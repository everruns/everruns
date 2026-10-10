// New agent page: every path ends in a normal agent. The
// builder and the Blank form share one draft shape so they create agents the
// same way: the agent first, then each way in as a draft channel, then the
// schedule. Nothing takes traffic until the user publishes a channel.

import type { AgentDraft, DraftChannelKind } from "@/lib/api/agent-draft";

export type NewAgentTab = "describe" | "example" | "blank" | "import";

export const NEW_AGENT_TABS: NewAgentTab[] = ["describe", "example", "blank", "import"];

export function parseNewAgentTab(value: string | null | undefined): NewAgentTab {
  return NEW_AGENT_TABS.includes(value as NewAgentTab) ? (value as NewAgentTab) : "describe";
}

export const DRAFT_CHANNEL_LABELS: Record<DraftChannelKind, { label: string; hint: string }> = {
  slack: { label: "Slack", hint: "Mentions and DMs in a workspace" },
  public_chat: { label: "Public chat", hint: "A hosted chat page with a link" },
  webhook: { label: "Webhook", hint: "Another system posts events" },
  ag_ui: { label: "AG-UI", hint: "Embed the agent in your app" },
};

export const DRAFT_CHANNEL_KINDS = Object.keys(DRAFT_CHANNEL_LABELS) as DraftChannelKind[];

/** Same rule as the server: `[a-z0-9]([a-z0-9-]*[a-z0-9])?`, at most 64 characters. */
export function slugifyAgentName(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64)
    .replace(/-+$/g, "");
}

export function isValidAgentSlug(value: string): boolean {
  return value.length <= 64 && /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(value);
}

/** What still stops the draft from being created, or null when it can be. */
export function draftBlocker(draft: AgentDraft): string | null {
  if (!draft.display_name.trim()) return "Give the agent a name";
  if (!isValidAgentSlug(draft.name)) {
    return "The addressable name takes lowercase letters, digits and hyphens";
  }
  if (!draft.system_prompt.trim()) return "Write the instructions";
  return null;
}

/**
 * Slack needs a workspace chosen or credentials entered, which the channel
 * form already handles, so a draft that asks for Slack lands there after the
 * agent exists instead of creating a half-configured channel.
 */
export function draftNeedsSlackSetup(draft: AgentDraft): boolean {
  return draft.channels.includes("slack");
}

export function newAgentLanding(agentId: string, draft: AgentDraft): string {
  return draftNeedsSlackSetup(draft)
    ? `/agents/${agentId}/channels/new?kind=slack`
    : `/agents/${agentId}`;
}

/** Message a webhook-started run receives when the draft does not say. */
export const DEFAULT_WEBHOOK_MESSAGE = "Handle the incoming webhook event.";
