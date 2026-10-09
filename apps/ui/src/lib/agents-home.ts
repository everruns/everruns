// Agents home: the pure parts of the page, kept out of the components so they
// are testable.
//
// Decision: "Needs attention" lists only problems a builder fixes in an agent's
// setup: a missing Slack permission or broken connection the health checker
// found, a default model that is no longer enabled, and a channel that is live
// to anyone without sign-in. Runtime noise (a failed run, the org's active-turn
// limit) stays out, so the list is short enough to act on.
//
// Decision: the page speaks in channel words. "Exposures" is retired; a channel
// is Live, Draft, Paused (turned off on the channel or on its agent) or "Agent
// archived", and "public access" means reachable without sign-in.

import type { OrgExposure } from "@/hooks/use-org-exposures";
import type {
  Agent,
  AgentActivity,
  AgentChannel,
  ChannelType,
  HealthIssue,
  PublicChatChannelConfig,
  SlackChannelConfig,
} from "@/lib/api/types";
import { getDisplayName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { formatRelativeTime } from "@/lib/formatting";

/* ───────────────────────────── Channel words ───────────────────────────── */

export type ChannelState = "live" | "draft" | "paused" | "agent-archived";

export function channelState(exposure: Pick<OrgExposure, "state">): ChannelState {
  switch (exposure.state) {
    case "live":
      return "live";
    case "draft":
      return "draft";
    case "disabled":
    case "suspended":
      return "paused";
    case "agent-inactive":
      return "agent-archived";
  }
}

export function channelStateLabel(state: ChannelState): string {
  switch (state) {
    case "live":
      return "Live";
    case "draft":
      return "Draft";
    case "paused":
      return "Paused";
    case "agent-archived":
      return "Agent archived";
  }
}

/** Short channel name for dense rows. */
export function channelShortName(kind: ChannelType): string {
  switch (kind) {
    case "a2a":
      return "A2A";
    case "fcp":
      return "FCP";
    case "ag_ui":
      return "AG-UI";
    case "api_endpoint":
      return "API channel";
    case "public_chat":
      return "Public Chat";
    case "slack":
      return "Slack";
    case "webhook":
      return "Webhook";
    case "schedule":
      return "Schedule";
    case "voice":
      return "Voice";
  }
}

/** Where a channel points, when its config says: a Slack channel, a chat's name. */
export function channelContext(channel: AgentChannel): string | null {
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return config.channel_id ?? null;
  }
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    return config.branding?.display_name?.trim() || null;
  }
  return null;
}

/** Channels list order: open doors first, then live, then by recent traffic. */
export function sortChannels(
  exposures: OrgExposure[],
  sessionsByChannel: Map<string, number>,
): OrgExposure[] {
  const rank = (exposure: OrgExposure) => {
    if (exposure.publiclyReachable) return 0;
    const state = channelState(exposure);
    if (state === "live") return 1;
    if (state === "paused") return 2;
    if (state === "draft") return 3;
    return 4;
  };
  return [...exposures].sort((a, b) => {
    const byRank = rank(a) - rank(b);
    if (byRank !== 0) return byRank;
    const byTraffic =
      (sessionsByChannel.get(b.channel.id) ?? 0) - (sessionsByChannel.get(a.channel.id) ?? 0);
    if (byTraffic !== 0) return byTraffic;
    return new Date(b.lastInvokedAt ?? 0).getTime() - new Date(a.lastInvokedAt ?? 0).getTime();
  });
}

/* ──────────────────────────── Needs attention ──────────────────────────── */

export type AttentionKind = "model" | "permission" | "connection" | "public-access";

export interface AttentionItem {
  key: string;
  kind: AttentionKind;
  /** Short category shown before the title. */
  label: string;
  title: string;
  body: string;
  agentId: string;
  agentName: string;
  /** Warnings degrade the agent; errors stop part of it. */
  severity: "error" | "warning";
  /** Small trailing fact, such as when evidence was last checked. */
  detail?: string;
  healthIssueId?: string;
  channelId?: string;
}

interface AttentionInputs {
  agents: Agent[];
  exposures: OrgExposure[];
  healthIssues: HealthIssue[];
  /** Models keyed by id. Only a known, disabled model is reported. */
  models: Map<string, { enabled: boolean; display_name: string }>;
}

const KIND_LABEL: Record<AttentionKind, string> = {
  model: "Model",
  permission: "Permission",
  connection: "Connection",
  "public-access": "Public access",
};

export function attentionItems({
  agents,
  exposures,
  healthIssues,
  models,
}: AttentionInputs): AttentionItem[] {
  const active = agents.filter((agent) => !isArchivedStatus(agent.status));
  const activeIds = new Set(active.map((agent) => agent.id));
  const items: AttentionItem[] = [];

  for (const agent of active) {
    const model = agent.default_model_id ? models.get(agent.default_model_id) : undefined;
    if (model && !model.enabled) {
      items.push({
        key: `model:${agent.id}`,
        kind: "model",
        label: KIND_LABEL.model,
        title: "Default model is no longer enabled",
        body: `${model.display_name} is disabled for this organization, so new sessions cannot start on it.`,
        agentId: agent.id,
        agentName: getDisplayName(agent),
        severity: "error",
      });
    }
  }

  for (const issue of healthIssues) {
    // Organization-level issues (the active-turn limit) are capacity, not setup.
    if (!issue.agent_id || !activeIds.has(issue.agent_id)) continue;
    if (issue.status !== "open" && issue.status !== "needs_check") continue;
    const kind: AttentionKind = issue.missing_scopes.length > 0 ? "permission" : "connection";
    items.push({
      key: `health:${issue.id}`,
      kind,
      label: KIND_LABEL[kind],
      title: issue.title,
      body: issue.body,
      agentId: issue.agent_id,
      agentName: issue.agent_name ?? "",
      severity: kind === "permission" ? "warning" : "error",
      detail: `Checked ${formatRelativeTime(issue.last_checked_at)}`,
      healthIssueId: issue.id,
      channelId: issue.channel_id ?? undefined,
    });
  }

  for (const exposure of exposures) {
    if (!exposure.publiclyReachable || !exposure.agent) continue;
    const name = channelShortName(exposure.channel.channel_type);
    items.push({
      key: `public:${exposure.channel.id}`,
      kind: "public-access",
      label: KIND_LABEL["public-access"],
      title: `${name} is reachable without sign-in`,
      body: "Live and anonymous. Anyone with the link can start a session.",
      agentId: exposure.agent.id,
      agentName: getDisplayName(exposure.agent),
      severity: "error",
      channelId: exposure.channel.id,
    });
  }

  const order = (item: AttentionItem) => (item.severity === "error" ? 0 : 1);
  return items.sort((a, b) => order(a) - order(b));
}

/* ─────────────────────────────── Agent rows ────────────────────────────── */

export type AgentTab = "all" | "running" | "attention" | "idle" | "archived";

export interface AgentNow {
  tone: "running" | "idle" | "paused" | "archived";
  label: string;
}

export function agentNow(agent: Agent, activity: AgentActivity | undefined): AgentNow {
  if (isArchivedStatus(agent.status)) return { tone: "archived", label: "Archived" };
  if (agent.exposures_suspended) return { tone: "paused", label: "Channels paused" };
  const running = activity?.running_sessions ?? 0;
  if (running > 0) return { tone: "running", label: `${running} running` };
  if (activity?.last_turn_at) {
    return { tone: "idle", label: `Idle · last run ${formatRelativeTime(activity.last_turn_at)}` };
  }
  return { tone: "idle", label: "Never run" };
}

export function matchesAgentTab(
  tab: AgentTab,
  agent: Agent,
  activity: AgentActivity | undefined,
  hasAttention: boolean,
): boolean {
  const archived = isArchivedStatus(agent.status);
  switch (tab) {
    case "all":
      return !archived;
    case "archived":
      return archived;
    case "running":
      return !archived && (activity?.running_sessions ?? 0) > 0;
    case "attention":
      return !archived && hasAttention;
    case "idle":
      return !archived && (activity?.running_sessions ?? 0) === 0;
  }
}

/** Busy and troubled agents first, then by recent runs, then by name. */
export function sortAgents(
  agents: Agent[],
  activityById: Map<string, AgentActivity>,
  attentionByAgent: Map<string, AttentionItem[]>,
): Agent[] {
  return [...agents].sort((a, b) => {
    const running = (agent: Agent) => activityById.get(agent.id)?.running_sessions ?? 0;
    const byRunning = Number(running(b) > 0) - Number(running(a) > 0);
    if (byRunning !== 0) return byRunning;
    const attention = (agent: Agent) => Number((attentionByAgent.get(agent.id)?.length ?? 0) > 0);
    const byAttention = attention(b) - attention(a);
    if (byAttention !== 0) return byAttention;
    const runs = (agent: Agent) => activityById.get(agent.id)?.runs ?? 0;
    const byRuns = runs(b) - runs(a);
    if (byRuns !== 0) return byRuns;
    return getDisplayName(a).localeCompare(getDisplayName(b));
  });
}

export function initials(name: string): string {
  const words = name
    .trim()
    .split(/[\s_-]+/)
    .filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 2);
  return letters.toUpperCase();
}
