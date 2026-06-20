// EVE-581: the Tasks tab exposes a per-task drill-down (lifecycle + message
// thread). These tests mock the task hooks and assert the drill-down renders the
// thread on demand.
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import type { SessionTask, SessionTaskDetail } from "@/lib/api/types";

const mockUseSessionTasks = jest.fn();
const mockUseSessionTask = jest.fn();
const mockUseCancelSessionTask = jest.fn(() => ({ mutate: jest.fn(), isPending: false }));
const mockUseSendTaskMessage = jest.fn(() => ({
  mutate: jest.fn(),
  isPending: false,
  isError: false,
  error: null,
}));
const mockUseSessionResources = jest.fn(() => ({ data: [], isLoading: false, error: null }));

jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTasks: (...args: unknown[]) => mockUseSessionTasks(...args),
  useSessionTask: (...args: unknown[]) => mockUseSessionTask(...args),
  useCancelSessionTask: (...args: unknown[]) => mockUseCancelSessionTask(...args),
  useSendTaskMessage: (...args: unknown[]) => mockUseSendTaskMessage(...args),
}));

jest.mock("@/hooks/use-session-resources", () => ({
  useSessionResources: (...args: unknown[]) => mockUseSessionResources(...args),
}));

jest.mock("../app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => ({ sessionId: "session-1" }),
}));

// eslint-disable-next-line import/first
import ResourcesPage from "../app/(main)/sessions/[sessionId]/resources/page";

const baseTask: SessionTask = {
  id: "task_abc",
  session_id: "session-1",
  kind: "subagent",
  display_name: "Test Runner",
  state: "running",
  attempt: 1,
  wake_policy: "always",
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
} as SessionTask;

const detail: SessionTaskDetail = {
  task: baseTask,
  messages: [
    {
      id: "tmsg_1",
      task_id: "task_abc",
      direction: "outbound",
      content: [{ type: "text", text: "Working on the test suite" }],
      created_at: new Date().toISOString(),
    },
    {
      id: "tmsg_2",
      task_id: "task_abc",
      direction: "inbound",
      content: [{ type: "text", text: "Focus on the failing case" }],
      created_at: new Date().toISOString(),
    },
  ],
};

beforeEach(() => {
  jest.clearAllMocks();
  mockUseSessionTasks.mockReturnValue({ data: [baseTask], isLoading: false, error: null });
  mockUseSessionTask.mockReturnValue({ data: detail, isLoading: false, error: null });
});

describe("Tasks tab drill-down", () => {
  it("shows the task summary but hides the thread until expanded", () => {
    render(<ResourcesPage />);
    expect(screen.getByText("Test Runner")).toBeInTheDocument();
    // Thread content is not rendered before expansion.
    expect(screen.queryByText("Working on the test suite")).not.toBeInTheDocument();
    expect(screen.queryByText("Message thread")).not.toBeInTheDocument();
  });

  it("reveals the lifecycle timeline and bidirectional thread when expanded", () => {
    render(<ResourcesPage />);
    fireEvent.click(screen.getByRole("button", { name: /show task details/i }));

    expect(screen.getByText("Message thread")).toBeInTheDocument();
    expect(screen.getByText("Working on the test suite")).toBeInTheDocument();
    expect(screen.getByText("Focus on the failing case")).toBeInTheDocument();
    // Direction labels distinguish inbound vs outbound.
    expect(screen.getByText(/Session → task/)).toBeInTheDocument();
    expect(screen.getByText(/Task → session/)).toBeInTheDocument();
  });

  it("shows an empty-thread message when a task has no messages", () => {
    mockUseSessionTask.mockReturnValue({
      data: { task: baseTask, messages: [] },
      isLoading: false,
      error: null,
    });
    render(<ResourcesPage />);
    fireEvent.click(screen.getByRole("button", { name: /show task details/i }));
    expect(screen.getByText(/No messages exchanged with this task yet/i)).toBeInTheDocument();
  });
});
