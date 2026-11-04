// Background limits on unattended work (schedule lifetime, schedule budgets, the
// org's daily unattended cap). Augments the interfaces legacy-api-types.ts
// exports (including the organization types it re-exports) rather than growing
// it: that file is on the source-size ratchet.
import type {
  BackgroundLimits,
  BudgetStatus,
  BudgetSubjectType,
  SelfHealingPolicy,
} from "./schema-types";

/** Why the platform stopped a schedule; null while it runs or after a person re-enables it. */
export type ScheduleStopReason = "ended" | "budget_exhausted" | "runaway";

/** The schedule's own budget, as the schedule list reports it. */
export interface ScheduleBudget {
  limit_usd: number;
  spent_usd: number;
  status: BudgetStatus;
}

/** Budget subjects the limits page labels. */
export type BackgroundBudgetSubjectType = BudgetSubjectType | "session_schedule" | "org_unattended";

declare module "./legacy-api-types" {
  interface Organization {
    background_limits?: BackgroundLimits;
    /** What Everruns does on its own when a finding opens (self-healing). */
    self_healing?: SelfHealingPolicy;
  }
  interface UpdateOrganizationRequest {
    /** Admin only; replaces the whole object. */
    background_limits?: BackgroundLimits;
    /** Admin only; replaces the whole object. */
    self_healing?: SelfHealingPolicy;
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
