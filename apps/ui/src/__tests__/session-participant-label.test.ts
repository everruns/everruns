import { getSessionParticipantLabel } from "@/lib/session-participant-label";
import type { SessionParticipant } from "@/lib/api/types";

function participant(overrides: Partial<SessionParticipant> = {}): SessionParticipant {
  return {
    id: "part_01933b5a00007000800000000000001",
    session_id: "session_01933b5a00007000800000000000001",
    kind: "user",
    principal_id: "principal_01933b5a000070008000000000000001",
    role: "member",
    joined_at: "2026-08-06T12:00:00Z",
    ...overrides,
  };
}

describe("getSessionParticipantLabel", () => {
  it("uses the participant's persisted display name", () => {
    expect(getSessionParticipantLabel(participant({ display_name: "Mykhailo Chalyi" }))).toBe(
      "Mykhailo Chalyi",
    );
  });

  it("preserves an explicit name ahead of an agent lookup", () => {
    const agentNames = new Map([["agent_1", "Current agent name"]]);
    expect(
      getSessionParticipantLabel(
        participant({ kind: "agent", agent_id: "agent_1", display_name: "Invited expert" }),
        agentNames,
      ),
    ).toBe("Invited expert");
  });

  it("uses non-misleading kind fallbacks for missing or blank names", () => {
    expect(getSessionParticipantLabel(participant({ display_name: "  " }))).toBe("User");
    expect(getSessionParticipantLabel(participant({ kind: "agent" }))).toBe("Agent");
  });
});
