// Org Home: one ranked "Needs attention" list merged from the signals each
// owning page already computes (knowledge/ui/org-home.md). Home adds no detector
// of its own except the failure burst, which reads the agents activity buckets.
// Every item links to the page where it is fixed; snoozing stays on that page.

import type { AttentionItem } from "@/lib/agents-home";
import type {
  AgentActivity,
  DoctorReport,
  HealthIssue,
  SandboxFleetStats,
  Session,
} from "@/lib/api/types";

export type HomeSeverity = "error" | "warning" | "info";
export type HomeSource = "agents" | "health" | "sessions" | "sandboxes" | "system";

export interface HomeAttentionItem {
  key: string;
  source: HomeSource;
  severity: HomeSeverity;
  /** Product copy is a translation template filled from `values`; server copy is passed through. */
  title: string;
  body: string;
  values?: Record<string, string | number>;
  /** Agent, session or channel the item is about, shown before the body. */
  subject?: string;
  href: string;
  action: string;
  /** When the condition started, for ordering and the age column. */
  since?: string;
}

/** Turns in the burst window before a failure share is judged at all. */
export const BURST_MIN_RUNS = 3;
/** Share of failed turns in the burst window that makes an agent "failing". */
export const BURST_FAILURE_SHARE = 0.5;
/** Hourly buckets read for a burst: the current hour and the one before. */
export const BURST_HOURS = 2;

const AGENT_ACTION: Record<AttentionItem["kind"], string> = {
  model: "Pick model",
  permission: "Reconnect",
  connection: "Reconnect",
  "public-access": "Review",
  setup: "Finish setup",
};

interface HomeInputs {
  /** Items from the Agents page model (`attentionItems`). */
  agentItems: AttentionItem[];
  healthIssues: HealthIssue[];
  /** Sessions paused at their usage limit. */
  pausedSessions: Session[];
  pausedTotal: number;
  activity: AgentActivity[];
  agentNames: Map<string, string>;
  sandboxStats?: SandboxFleetStats;
  /** System doctor report; only passed for viewers allowed to see System. */
  doctor?: DoctorReport;
  now?: number;
}

function snoozed(issue: HealthIssue, now: number): boolean {
  return !!issue.snoozed_until && Date.parse(issue.snoozed_until) > now;
}

function isOpen(issue: HealthIssue): boolean {
  return issue.status === "open" || issue.status === "needs_check";
}

/** Open health issues the viewer has not snoozed. */
export function countOpenIssues(issues: HealthIssue[], now = Date.now()): number {
  return issues.filter((issue) => isOpen(issue) && !snoozed(issue, now)).length;
}

/** Agents whose recent turns mostly failed. */
export function failureBursts(activity: AgentActivity[]) {
  const bursts: Array<{ agentId: string; runs: number; failed: number }> = [];
  for (const agent of activity) {
    const recent = agent.hourly.slice(-BURST_HOURS);
    const runs = recent.reduce((sum, bucket) => sum + bucket.runs, 0);
    const failed = recent.reduce((sum, bucket) => sum + bucket.failed, 0);
    if (runs >= BURST_MIN_RUNS && failed / runs >= BURST_FAILURE_SHARE) {
      bursts.push({ agentId: agent.agent_id, runs, failed });
    }
  }
  return bursts.sort((a, b) => b.failed - a.failed);
}

export function homeAttentionItems(inputs: HomeInputs): HomeAttentionItem[] {
  const now = inputs.now ?? Date.now();
  const items: HomeAttentionItem[] = [];
  const coveredIssues = new Set(
    inputs.agentItems.map((item) => item.healthIssueId).filter(Boolean) as string[],
  );

  for (const item of inputs.agentItems) {
    const issue = item.healthIssueId
      ? inputs.healthIssues.find((candidate) => candidate.id === item.healthIssueId)
      : undefined;
    if (issue && snoozed(issue, now)) continue;
    items.push({
      key: `agent:${item.key}`,
      source: "agents",
      severity: item.severity,
      title: item.title,
      body: item.body,
      subject: item.agentName,
      href: issue?.href ?? `/agents/${item.agentId}`,
      action: AGENT_ACTION[item.kind],
      since: issue?.first_detected_at,
    });
  }

  // Organization-level and background-work issues: the turn limit, failing
  // schedules, stuck sessions. Channel issues already came through the agents.
  for (const issue of inputs.healthIssues) {
    if (coveredIssues.has(issue.id) || snoozed(issue, now)) continue;
    if (!isOpen(issue)) continue;
    items.push({
      key: `health:${issue.id}`,
      source: "health",
      severity: issue.code.startsWith("org.") ? "warning" : "error",
      title: issue.title,
      body: issue.body,
      subject: issue.agent_name ?? undefined,
      href: issue.href,
      action: "Open",
      since: issue.first_detected_at,
    });
  }

  for (const burst of failureBursts(inputs.activity)) {
    const name = inputs.agentNames.get(burst.agentId);
    if (name === undefined) continue;
    items.push({
      key: `burst:${burst.agentId}`,
      source: "agents",
      severity: "error",
      title: "{failed} of {runs} turns failed in the last {hours} hours",
      values: { failed: burst.failed, runs: burst.runs, hours: BURST_HOURS },
      body: "Most recent turns ended in an error. Open the failed sessions to see why.",
      subject: name,
      href: `/sessions?status=failed&agent=${encodeURIComponent(burst.agentId)}`,
      action: "Open sessions",
    });
  }

  if (inputs.pausedTotal > 0) {
    const named = inputs.pausedSessions
      .slice(0, 2)
      .map((session) => session.title || session.id)
      .join(", ");
    const more = inputs.pausedTotal > 2 ? ", …" : "";
    const oldest = inputs.pausedSessions
      .map((session) => session.updated_at)
      .filter(Boolean)
      .sort()[0];
    items.push({
      key: "sessions:paused",
      source: "sessions",
      severity: "error",
      title:
        inputs.pausedTotal === 1
          ? "1 session paused at its usage limit"
          : "{count} sessions paused at their usage limit",
      body: "{sessions}. A paused run resumes when its limit is raised.",
      values: { count: inputs.pausedTotal, sessions: `${named}${more}` },
      href: "/sessions?status=paused",
      action: "Review",
      since: oldest,
    });
  }

  const sandboxes = inputs.sandboxStats?.needs_attention ?? 0;
  if (sandboxes > 0) {
    items.push({
      key: "sandboxes:attention",
      source: "sandboxes",
      severity: "warning",
      title: sandboxes === 1 ? "1 sandbox needs attention" : "{count} sandboxes need attention",
      values: { count: sandboxes },
      body: "Lost, failed to start, failed cleanup, or running with no activity for an hour.",
      href: "/sandboxes?state=all&attention=1",
      action: "Open fleet",
    });
  }

  for (const check of inputs.doctor?.checks ?? []) {
    if (check.status !== "problem" && check.status !== "warning") continue;
    items.push({
      key: `system:${check.id}`,
      source: "system",
      severity: check.status === "problem" ? "error" : "warning",
      title: check.title,
      body: check.summary,
      href: "/system/health",
      action: "Open",
      since: check.observed_at,
    });
  }

  return rankAttention(items);
}

const SEVERITY_ORDER: Record<HomeSeverity, number> = { error: 0, warning: 1, info: 2 };

/** Blocking first; within a severity, the condition that started earliest. */
export function rankAttention(items: HomeAttentionItem[]): HomeAttentionItem[] {
  return [...items].sort((a, b) => {
    const severity = SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity];
    if (severity) return severity;
    const at = (item: HomeAttentionItem) => (item.since ? Date.parse(item.since) : Infinity);
    return at(a) - at(b);
  });
}

/** Org-wide turns started and failed per hour, summed across agents. */
export function hourlyTotals(activity: AgentActivity[]) {
  const length = Math.max(0, ...activity.map((agent) => agent.hourly.length));
  return Array.from({ length }, (_, index) =>
    activity.reduce(
      (sum, agent) => {
        // Agents' buckets all end at `generated_at`; align from the right.
        const bucket = agent.hourly[agent.hourly.length - length + index];
        return bucket ? { runs: sum.runs + bucket.runs, failed: sum.failed + bucket.failed } : sum;
      },
      { runs: 0, failed: 0 },
    ),
  );
}

/** Most active agents first; idle ones by their latest turn. */
export function busiestAgents(activity: AgentActivity[], limit: number): AgentActivity[] {
  return [...activity]
    .sort(
      (a, b) =>
        b.running_sessions - a.running_sessions ||
        b.runs - a.runs ||
        (b.last_turn_at ?? "").localeCompare(a.last_turn_at ?? ""),
    )
    .slice(0, limit);
}
