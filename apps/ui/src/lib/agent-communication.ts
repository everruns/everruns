// Agent-level Communication setting. It replaced Slack's per-channel reply mode:
// how an agent talks is a property of the agent, honored on every surface.
import type { Communication } from "@/lib/api/types";

export const DEFAULT_COMMUNICATION: Communication = "direct";

export const COMMUNICATION_OPTIONS: {
  value: Communication;
  label: string;
  description: string;
}[] = [
  {
    value: "direct",
    label: "Direct",
    description: "Replies are the agent's text, shown as it writes.",
  },
  {
    value: "explicit",
    label: "Explicit",
    description:
      "The agent's text is private notes. It talks only through send_message and can choose not to reply. Best for Slack channels, coordinators and multi-person conversations.",
  },
];

export function normalizeCommunication(value: string | null | undefined): Communication {
  return value === "explicit" ? "explicit" : DEFAULT_COMMUNICATION;
}

export function getCommunicationOption(value: string | null | undefined) {
  const normalized = normalizeCommunication(value);
  return COMMUNICATION_OPTIONS.find((option) => option.value === normalized)!;
}
