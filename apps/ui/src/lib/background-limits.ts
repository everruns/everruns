// Shared constants and labels for background limits. The defaults and ceilings
// mirror the server's code constants; the server stays authoritative and
// rejects anything outside the ceilings.
import type { BackgroundBudgetSubjectType, BackgroundLimits } from "@/lib/api/types";

export const DEFAULT_BACKGROUND_LIMITS: BackgroundLimits = {
  schedule_max_lifetime_days: 7,
  schedule_max_runs: 96,
  schedule_budget_usd: 2,
  schedule_max_budget_usd: 10,
  org_unattended_daily_usd: 20,
};

/** Platform ceilings an org cannot exceed for agent-created schedules. */
export const BACKGROUND_LIMIT_CEILINGS = {
  schedule_max_lifetime_days: 7,
  schedule_max_runs: 96,
} as const;

/** English source labels for budget subjects; pass the result through `ui()`. */
const BUDGET_SUBJECT_LABELS: Record<BackgroundBudgetSubjectType, string> = {
  session: "Session",
  agent: "Agent",
  user: "User",
  organization: "Organization",
  agent_trigger: "Trigger",
  agent_channel: "Channel",
  session_schedule: "Schedule",
  org_unattended: "Unattended work (daily)",
};

export function budgetSubjectLabel(subjectType: string): string {
  return BUDGET_SUBJECT_LABELS[subjectType as BackgroundBudgetSubjectType] ?? subjectType;
}

/** Older servers omit the field; fill in the code defaults so the form always has values. */
export function effectiveBackgroundLimits(limits: BackgroundLimits | undefined): BackgroundLimits {
  return { ...DEFAULT_BACKGROUND_LIMITS, ...limits };
}
