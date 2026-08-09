// The Work tab of a session recording (EVE-854). It replaces the org-wide
// `/work` view (EVE-756) and the old Tasks/Resources tab: same chip selector,
// same shared `TaskCard` detail with its per-task drill-down (EVE-581), scoped
// to one session and read-only throughout.

import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import type { SessionTask, SessionTaskDetail } from "@/lib/api/types";

const mockUseSessionTasks = jest.fn();
const mockUseSessionTask = jest.fn();
const mockUseSessionResources = jest.fn(() => ({ data: [], isLoading: false, error: null }));
const mockUseSessionSchedules = jest.fn<
  { data: unknown[]; isLoading: boolean; error: unknown },
  []
>(() => ({ data: [], isLoading: false, error: null }));

jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: () => mockUseSessionTasks(),
  useSessionTask: () => mockUseSessionTask(),
}));

jest.mock("@/hooks/use-session-resources", () => ({
  useSessionResources: () => mockUseSessionResources(),
}));

jest.mock("@/hooks/use-session-schedules", () => ({
  useSessionSchedules: () => mockUseSessionSchedules(),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    // eslint-disable-next-line nextjs/no-html-link-for-pages
    <a href={href}>{children}</a>
  ),
}));

jest.mock("../app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => ({
    sessionId: "session_1",
    session: { id: "session_1", features: ["leased_resources", "schedules"] },
  }),
}));

// eslint-disable-next-line import/first
import SessionWorkPage from "../app/(main)/sessions/[sessionId]/work/page";

function makeTask(overrides: Partial<SessionTask>): SessionTask {
  return {
    id: "task_x",
    session_id: "session_1",
    kind: "subagent",
    display_name: "Task",
    state: "running",
    attempt: 1,
    wake_policy: "always",
    created_at: "2026-05-25T10:00:00Z",
    updated_at: "2026-05-25T10:00:00Z",
    ...overrides,
  } as unknown as SessionTask;
}

const detail: SessionTaskDetail = {
  task: makeTask({ id: "task_1", display_name: "Test Runner" }),
  messages: [
    {
      id: "tmsg_1",
      task_id: "task_1",
      direction: "outbound",
      content: [{ type: "text", text: "Working on the test suite" }],
      created_at: "2026-05-25T10:01:00Z",
    },
    {
      id: "tmsg_2",
      task_id: "task_1",
      direction: "inbound",
      content: [{ type: "text", text: "Focus on the failing case" }],
      created_at: "2026-05-25T10:02:00Z",
    },
  ],
} as unknown as SessionTaskDetail;

beforeEach(() => {
  jest.clearAllMocks();
  mockUseSessionTasks.mockReturnValue({
    data: [makeTask({ id: "task_1", display_name: "Test Runner" })],
    isLoading: false,
    error: null,
  });
  mockUseSessionTask.mockReturnValue({ data: detail, isLoading: false, error: null });
  mockUseSessionResources.mockReturnValue({ data: [], isLoading: false, error: null });
  mockUseSessionSchedules.mockReturnValue({ data: [], isLoading: false, error: null });
});

describe("session Work tab", () => {
  it("lists this session's tasks as chips and opens the shared detail card", () => {
    render(<SessionWorkPage />);

    expect(screen.getByText(/Select a task to see its detail/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Test Runner/ }));

    expect(screen.getByRole("button", { name: /show task details/i })).toBeInTheDocument();
    expect(screen.queryByText(/Select a task to see its detail/)).not.toBeInTheDocument();
  });

  it("renders a task whose kind is unknown at build time", () => {
    mockUseSessionTasks.mockReturnValue({
      data: [makeTask({ id: "task_u", display_name: "Mystery", kind: "quantum_widget" })],
      isLoading: false,
      error: null,
    });
    render(<SessionWorkPage />);

    expect(screen.getByRole("button", { name: /Mystery/ })).toBeInTheDocument();
  });

  it("hides the task drill-down until it is expanded", () => {
    render(<SessionWorkPage />);
    fireEvent.click(screen.getByRole("button", { name: /Test Runner/ }));

    expect(screen.queryByText("Working on the test suite")).not.toBeInTheDocument();
    expect(screen.queryByText("Message thread")).not.toBeInTheDocument();
  });

  it("reveals the lifecycle timeline and bidirectional thread when expanded", () => {
    render(<SessionWorkPage />);
    fireEvent.click(screen.getByRole("button", { name: /Test Runner/ }));
    fireEvent.click(screen.getByRole("button", { name: /show task details/i }));

    expect(screen.getByText("Message thread")).toBeInTheDocument();
    expect(screen.getByText("Working on the test suite")).toBeInTheDocument();
    expect(screen.getByText("Focus on the failing case")).toBeInTheDocument();
    expect(screen.getByText(/Session → task/)).toBeInTheDocument();
    expect(screen.getByText(/Task → session/)).toBeInTheDocument();
  });

  it("shows what a waiting task asked for without offering to answer it", () => {
    mockUseSessionTasks.mockReturnValue({
      data: [
        makeTask({
          id: "task_wait",
          display_name: "Reviewer",
          state: "awaiting_input",
          input_request: { id: "req_1", prompt: "Which branch should I review?" },
        }),
      ],
      isLoading: false,
      error: null,
    });
    render(<SessionWorkPage />);
    fireEvent.click(screen.getByRole("button", { name: /Reviewer/ }));

    expect(screen.getByText("Which branch should I review?")).toBeInTheDocument();
    expect(screen.getByText(/Fork this session into a chat to answer/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^send$/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^cancel$/i })).not.toBeInTheDocument();
  });

  it("shows schedules as inert facts, with no enable, trigger or delete control", () => {
    mockUseSessionSchedules.mockReturnValue({
      data: [
        {
          id: "sched_1",
          session_id: "session_1",
          schedule_type: "recurring",
          cron_expression: "0 9 * * *",
          timezone: "UTC",
          enabled: true,
          trigger_count: 2,
          description: "Daily digest",
          created_at: "2026-05-25T10:00:00Z",
          updated_at: "2026-05-25T10:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    });
    render(<SessionWorkPage />);

    expect(screen.getByText("Daily digest")).toBeInTheDocument();
    expect(screen.getByText("Active")).toBeInTheDocument();
    for (const control of [/disable schedule/i, /trigger now/i, /delete schedule/i]) {
      expect(screen.queryByRole("button", { name: control })).not.toBeInTheDocument();
    }
  });

  it("shows an empty state when the session started no work", () => {
    mockUseSessionTasks.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<SessionWorkPage />);

    expect(screen.getByText(/No work yet/)).toBeInTheDocument();
  });
});
