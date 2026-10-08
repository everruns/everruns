import {
  assignmentGroup,
  checklistSummary,
  conversationGroup,
  coordinatorThreads,
  currentAssignmentLabel,
  threadRowTitle,
  workGroup,
} from "@/lib/chat-thread-work";
import type { Session, SessionTask } from "@/lib/api/types";

describe("thread work grouping", () => {
  it("keeps idle conversations open until explicitly resolved", () => {
    expect(conversationGroup({ status: "idle", archived_at: null } as Session)).toBe("Open");
    expect(
      conversationGroup({ status: "idle", archived_at: "2026-10-03T00:00:00Z" } as Session),
    ).toBe("Resolved");
    expect(
      conversationGroup({ status: "active", archived_at: "2026-10-03T00:00:00Z" } as Session),
    ).toBe("Working");
    expect(conversationGroup({ status: "waiting_for_tool_results" } as Session)).toBe("Needs you");
  });
  it.each([
    ["queued", "Working"],
    ["running", "Working"],
    ["awaiting_input", "Needs you"],
    ["failed", "Needs you"],
    ["succeeded", "Resolved"],
    ["canceled", "Resolved"],
  ])("preserves the task lifecycle: %s → %s", (state, expected) => {
    expect(workGroup({ state } as SessionTask)).toBe(expected);
  });

  it.each([
    [{ state: "running" }, "Working"],
    [{ state: "awaiting_input" }, "Needs you"],
    [{ state: "succeeded" }, "Ready for review"],
    [{ state: "succeeded", state_detail: "resolved" }, "Resolved"],
    [{ state: "canceled" }, "Open"],
  ])("groups a coordinator thread by its latest assignment: %o → %s", (task, expected) => {
    expect(assignmentGroup(task as SessionTask)).toBe(expected);
  });
  it("folds a thread's assignments into one entry led by the newest", () => {
    const assignment = (id: string, thread: string, created_at: string) =>
      ({
        id,
        kind: "assignment",
        state: "succeeded",
        created_at,
        links: { child_session_id: thread },
      }) as SessionTask;
    const threads = coordinatorThreads([
      assignment("task_1", "session_a", "2026-10-07T00:00:00Z"),
      assignment("task_2", "session_a", "2026-10-08T00:00:00Z"),
      assignment("task_3", "session_b", "2026-10-07T00:00:00Z"),
      { id: "task_bg", kind: "background", links: {} } as SessionTask,
    ]);
    expect(
      threads.map((thread) => [thread.threadId, thread.latest.id, thread.assignments]),
    ).toEqual([
      ["session_a", "task_2", 2],
      ["session_b", "task_3", 1],
    ]);
  });
  it("keeps the thread's first title and shows a follow-up assignment beside it", () => {
    const assignment = (id: string, display_name: string, created_at: string) =>
      ({
        id,
        display_name,
        kind: "assignment",
        state: "running",
        created_at,
        links: { child_session_id: "session_a" },
      }) as SessionTask;
    const [single] = coordinatorThreads([assignment("task_1", "Releases", "2026-10-07")]);
    expect(threadRowTitle(single)).toBe("Releases");
    expect(currentAssignmentLabel(single)).toBeNull();
    // Listed newest first, as the API returns them.
    const [followed] = coordinatorThreads([
      assignment("task_2", "Add release dates", "2026-10-08"),
      assignment("task_1", "Releases", "2026-10-07"),
    ]);
    expect(threadRowTitle(followed)).toBe("Releases");
    expect(currentAssignmentLabel(followed)).toBe("Add release dates");
  });
  it("summarizes the worker checklist", () => {
    expect(checklistSummary({} as SessionTask)).toBeNull();
    expect(
      checklistSummary({
        progress: {
          steps: [
            { title: "Read the code", status: "done" },
            { title: "Write the fix", status: "in_progress" },
            { title: "Open the PR", status: "pending" },
          ],
        },
      } as unknown as SessionTask),
    ).toBe("Write the fix · 1 of 3 steps done");
  });
});
