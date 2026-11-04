// Which health issues the Evals section shows as findings, and how loud each
// is. Codes come from the server's detectors and classifiers
// (crates/server/src/domains/health_issues/{behavior,classifiers,background_work}.rs).
// Classifier findings notify only; nothing is suspended on them.

import type { HealthIssue } from "@/lib/api/types";

export type FindingSeverity = "high" | "medium";

const FINDING_SEVERITY: Record<string, FindingSeverity> = {
  "session.suspected_abuse": "high",
  "session.followed_injection": "high",
  "schedule.runaway": "high",
  "session.agent_stuck": "medium",
  "session.tool_error_loop": "medium",
  "session.repeated_tool_calls": "medium",
  "session.spend_spike": "medium",
  "session.context_bloat": "medium",
  "agent.unattended_spend_spike": "medium",
  "observer.pass_rate_low": "medium",
};

export function isFinding(issue: HealthIssue): boolean {
  return issue.code in FINDING_SEVERITY;
}

export function findingSeverity(issue: HealthIssue): FindingSeverity {
  return FINDING_SEVERITY[issue.code] ?? "medium";
}

/** Findings, the loudest first, then the newest. */
export function sortFindings(issues: HealthIssue[]): HealthIssue[] {
  return issues.filter(isFinding).sort((a, b) => {
    const rank = (issue: HealthIssue) => ["high", "medium", "low"].indexOf(findingLevel(issue));
    return rank(a) - rank(b) || b.first_detected_at.localeCompare(a.first_detected_at);
  });
}

/** What a finding's code is, in a few words (mockup 04's kind tag). */
const FINDING_KIND: Record<string, string> = {
  "session.suspected_abuse": "Platform abuse",
  "session.followed_injection": "Injected instructions",
  "session.agent_stuck": "Agent stuck",
  "schedule.runaway": "Runaway work",
  "session.tool_error_loop": "Tool error loop",
  "session.repeated_tool_calls": "Repeated calls",
  "session.spend_spike": "Spend spike",
  "session.context_bloat": "Context growth",
  "agent.unattended_spend_spike": "Spend spike",
  "observer.pass_rate_low": "Quality drop",
};

export function findingKind(issue: HealthIssue): string {
  return FINDING_KIND[issue.code] ?? issue.code;
}

/** Severity the Inspector judged, else the detector's default. */
export function findingLevel(issue: HealthIssue): "high" | "medium" | "low" {
  const judged = issue.investigation?.severity;
  if (judged === "high" || judged === "medium" || judged === "low") return judged;
  return findingSeverity(issue);
}

export type EvidenceRow = { label: string; value: string };

const usd = (v: number) => `$${v < 1 ? v.toFixed(3) : v.toFixed(2)}`;

// How each recorded number reads. Keys come from the detectors' evidence
// (health_issues/{behavior,classifiers,background_work}.rs); unknown keys
// still show, with their name made readable.
const EVIDENCE: Record<string, { label: string; format?: (v: unknown) => string | null }> = {
  probability: { label: "Model's confidence", format: (v) => (v as number).toFixed(2) },
  model: { label: "Decision model" },
  reason: {
    label: "Read because",
    format: (v) => (v === "flagged" ? "a detector flagged it" : "random sample"),
  },
  runs: { label: "Runs" },
  stop_runs: { label: "Stops after" },
  stopped: { label: "Stopped", format: (v) => (v ? "yes" : "no") },
  // The actions card says what self-healing did.
  paused_by_policy: { label: "", format: () => null },
  trigger_count: { label: "Times fired" },
  limit_usd: { label: "Budget", format: (v) => usd(v as number) },
  spent_usd: { label: "Spent", format: (v) => usd(v as number) },
  last_day_usd: { label: "Last day", format: (v) => usd(v as number) },
  baseline_daily_usd: { label: "Normal per day", format: (v) => usd(v as number) },
  last_hour_usd: { label: "Last hour", format: (v) => usd(v as number) },
  baseline_hourly_usd: { label: "Normal per hour", format: (v) => usd(v as number) },
  tool: { label: "Tool" },
  calls: { label: "Calls" },
  failed: { label: "Failed" },
  window_calls: { label: "Calls looked at" },
  avg_prompt_tokens: {
    label: "Average prompt",
    format: (v) => `${Math.round(v as number).toLocaleString("en-US")} tokens`,
  },
  cached_percent: { label: "Cache hits", format: (v) => `${Math.round(v as number)}%` },
  samples: { label: "Days of history" },
  calibrated: { label: "", format: () => null },
};

/** The finding's recorded numbers as label and value rows. */
export function evidenceRows(evidence: unknown): EvidenceRow[] {
  if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) return [];
  const rows: EvidenceRow[] = [];
  for (const [key, raw] of Object.entries(evidence as Record<string, unknown>)) {
    if (raw === null || raw === undefined || typeof raw === "object") continue;
    const spec = EVIDENCE[key];
    const value = spec?.format ? spec.format(raw) : String(raw);
    if (value === null) continue;
    const label = spec?.label ?? key.replace(/_/g, " ").replace(/^\w/, (c) => c.toUpperCase());
    rows.push({ label, value });
  }
  return rows;
}

export type FindingGroup = { lead: HealthIssue; similar: number };

/**
 * One row per cluster the Inspector filed findings under, led by its loudest
 * and newest finding; findings without a cluster stand alone.
 */
export function groupFindings(issues: HealthIssue[]): FindingGroup[] {
  const groups: FindingGroup[] = [];
  const byCluster = new Map<string, FindingGroup>();
  for (const issue of sortFindings(issues)) {
    const cluster = issue.investigation?.cluster_id;
    const group = cluster ? byCluster.get(cluster) : undefined;
    if (group) {
      group.similar += 1;
      continue;
    }
    const created = { lead: issue, similar: 0 };
    groups.push(created);
    if (cluster) byCluster.set(cluster, created);
  }
  return groups;
}
