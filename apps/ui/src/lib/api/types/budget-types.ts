// Budget API types — mirrors `crates/core/src/budget.rs`.
// Behind the `app_budgets` feature flag for app/channel subjects.

export type BudgetSubjectType = "session" | "agent" | "user" | "org" | "app" | "app_channel";

export type BudgetStatus = "active" | "paused" | "exhausted" | "disabled";

/**
 * Period configuration for recurring budgets. Drives automatic balance reset.
 *  - `Duration` is a sliding window of `seconds` from `period_started_at`.
 *  - `Rolling` accepts shorthand like `5h`, `24h`, `30d` (server normalises).
 *  - `Calendar` aligns to UTC `hour | day | week | month | year` boundaries.
 */
export type BudgetPeriod =
  | { type: "duration"; seconds: number }
  | { type: "rolling"; window: string }
  | { type: "calendar"; unit: string };

export interface Budget {
  id: string;
  organization_id: string;
  subject_type: BudgetSubjectType;
  subject_id: string;
  currency: string;
  limit: number;
  soft_limit?: number | null;
  balance: number;
  period?: BudgetPeriod | null;
  period_started_at?: string | null;
  metadata?: Record<string, unknown> | null;
  status: BudgetStatus;
  created_at: string;
  updated_at: string;
}

export interface CreateBudgetRequest {
  subject_type: BudgetSubjectType;
  subject_id: string;
  currency: string;
  limit: number;
  soft_limit?: number | null;
  period?: BudgetPeriod | null;
  metadata?: Record<string, unknown> | null;
}

export interface UpdateBudgetRequest {
  limit?: number;
  soft_limit?: number | null;
  status?: BudgetStatus;
  metadata?: Record<string, unknown> | null;
}
