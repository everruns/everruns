// Background limits on unattended work (schedule lifetime, schedule budgets, the
// org's daily unattended cap). Augments the interfaces legacy-api-types.ts
// exports (including the organization types it re-exports) rather than growing
// it: that file is on the source-size ratchet.
import type { BudgetStatus, BudgetSubjectType } from "./schema-types";

/** Org-level limits on unattended work. PATCH replaces the whole object. */
export interface BackgroundLimits {
  /** Longest an agent-created schedule may run, in days (1..7). */
  schedule_max_lifetime_days: number;
  /** Most runs an agent-created schedule may have (1..96). */
  schedule_max_runs: number;
  /** Budget a schedule gets when the agent passes none, in USD. */
  schedule_budget_usd: number;
  /** Largest budget an agent may give a schedule, in USD. */
  schedule_max_budget_usd: number;
  /** Daily cap on all unattended work in the org, in USD; null means no cap. */
  org_unattended_daily_usd: number | null;
}

/** Why the platform stopped a schedule; null while it runs or after a person re-enables it. */
export type ScheduleStopReason = "ended" | "budget_exhausted";

/** The schedule's own budget, as the schedule list reports it. */
export interface ScheduleBudget {
  limit_usd: number;
  spent_usd: number;
  status: BudgetStatus;
}

/** Budget subjects, including the background-limit ones the generated enum may not list yet. */
export type BackgroundBudgetSubjectType = BudgetSubjectType | "session_schedule" | "org_unattended";

declare module "./legacy-api-types" {
  interface Organization {
    background_limits?: BackgroundLimits;
  }
  interface UpdateOrganizationRequest {
    /** Admin only; replaces the whole object. */
    background_limits?: BackgroundLimits;
  }
  interface SessionSchedule {
    ends_at?: string | null;
    max_runs?: number | null;
    /** When the work is done, in the agent's words. */
    stop_when?: string | null;
    stop_reason?: ScheduleStopReason | null;
    budget?: ScheduleBudget | null;
  }
  interface UpdateSessionScheduleRequest {
    /** A person may extend past the agent ceilings. */
    ends_at?: string | null;
    max_runs?: number | null;
    budget_usd?: number | null;
  }
}
