// EVE-756: the cross-session Work view groups org tasks into delegation trees by
// root_session_id, reuses the shared chip + detail card, and is kind-agnostic
// (an unknown task kind still renders from its snapshot).
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import type { SessionTask } from "@/lib/api/types";

const mockUseOrgTasks = jest.fn();
const mockUseSessionTask = jest.fn(() => ({ data: undefined, isLoading: false, error: null }));
const mockUseCancelSessionTask = jest.fn(() => ({ mutate: jest.fn(), isPending: false }));
const mockUseSendTaskMessage = jest.fn(() => ({
  mutate: jest.fn(),
  isPending: false,
  isError: false,
  error: null,
}));

jest.mock("@/hooks/use-org-tasks", () => ({
  useOrgTasks: () => mockUseOrgTasks(),
}));

// TaskCard (shared detail) reads these per-session hooks.
jest.mock("@/hooks/use-session-tasks", () => ({
  useSessionTask: () => mockUseSessionTask(),
  useCancelSessionTask: () => mockUseCancelSessionTask(),
  useSendTaskMessage: () => mockUseSendTaskMessage(),
}));

// eslint-disable-next-line import/first
import WorkPage from "../app/(main)/work/page";

function makeTask(overrides: Partial<SessionTask>): SessionTask {
  return {
    id: "task_x",
    session_id: "sess_root_aaaa1111",
    root_session_id: "sess_root_aaaa1111",
    kind: "subagent",
    display_name: "Task",
    state: "running",
    attempt: 1,
    wake_policy: "always",
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    ...overrides,
  } as unknown as SessionTask;
}

describe("Work view", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("groups tasks into delegation trees by root_session_id", () => {
    mockUseOrgTasks.mockReturnValue({
      data: [
        makeTask({
          id: "t_a",
          display_name: "Alpha",
          root_session_id: "sess_root_aaaa1111",
          session_id: "sess_root_aaaa1111",
        }),
        makeTask({
          id: "t_b",
          display_name: "Bravo",
          root_session_id: "sess_root_bbbb2222",
          session_id: "sess_root_bbbb2222",
        }),
      ],
      isLoading: false,
      error: null,
    });
    render(<WorkPage />);

    // One group header per distinct root.
    expect(screen.getByText(/aaaa1111/)).toBeInTheDocument();
    expect(screen.getByText(/bbbb2222/)).toBeInTheDocument();
    // Both tasks' chips render.
    expect(screen.getByRole("button", { name: /Alpha/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Bravo/ })).toBeInTheDocument();
  });

  it("renders a task whose kind is unknown at build time", () => {
    mockUseOrgTasks.mockReturnValue({
      data: [makeTask({ id: "t_u", display_name: "Mystery", kind: "quantum_widget" })],
      isLoading: false,
      error: null,
    });
    render(<WorkPage />);
    // Kind-agnostic: the chip still renders from the snapshot.
    expect(screen.getByRole("button", { name: /Mystery/ })).toBeInTheDocument();
  });

  it("nests descendant-session tasks under their root", () => {
    mockUseOrgTasks.mockReturnValue({
      data: [
        makeTask({
          id: "t_root",
          display_name: "Root work",
          session_id: "sess_root_aaaa1111",
          root_session_id: "sess_root_aaaa1111",
        }),
        makeTask({
          id: "t_child",
          display_name: "Child work",
          session_id: "sess_child_cccc3333",
          root_session_id: "sess_root_aaaa1111",
        }),
      ],
      isLoading: false,
      error: null,
    });
    render(<WorkPage />);
    // Both tasks appear under the single root, with the descendant labelled.
    expect(screen.getByText(/via session/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Root work/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Child work/ })).toBeInTheDocument();
  });

  it("opens the shared task detail when a chip is selected", () => {
    mockUseOrgTasks.mockReturnValue({
      data: [makeTask({ id: "t_sel", display_name: "Selectable" })],
      isLoading: false,
      error: null,
    });
    render(<WorkPage />);

    // Detail pane starts empty.
    expect(screen.getByText(/Select a task to see its detail/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Selectable/ }));

    // The shared TaskCard is now shown (its expand control appears).
    expect(screen.getByRole("button", { name: /show task details/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy ID: t_sel" })).toBeInTheDocument();
    expect(screen.queryByText(/Select a task to see its detail/)).not.toBeInTheDocument();
  });

  it("shows an empty state when there are no tasks", () => {
    mockUseOrgTasks.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<WorkPage />);
    expect(screen.getByText(/No work yet/)).toBeInTheDocument();
  });
});
