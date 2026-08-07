import type { SessionParticipant } from "@/lib/api/types";

export function getSessionParticipantLabel(
  participant: SessionParticipant,
  agentNameById?: ReadonlyMap<string, string>,
): string {
  const explicitName = participant.display_name?.trim();
  if (explicitName) return explicitName;
  if (participant.kind === "agent") {
    return (participant.agent_id && agentNameById?.get(participant.agent_id)) || "Agent";
  }
  return "User";
}
