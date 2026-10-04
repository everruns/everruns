import { conversationGroup, workGroup } from "@/lib/chat-thread-work";
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
});
