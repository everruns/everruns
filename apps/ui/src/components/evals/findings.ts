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
    const rank = (issue: HealthIssue) => (findingSeverity(issue) === "high" ? 0 : 1);
    return rank(a) - rank(b) || b.first_detected_at.localeCompare(a.first_detected_at);
  });
}
