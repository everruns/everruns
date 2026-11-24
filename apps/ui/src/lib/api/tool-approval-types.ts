// Tool approval policy, personal approval settings and the per-session mode
// switch (knowledge/execution/tool-approval.md, "Auto mode"). Augments the
// hand-written organization and event types rather than growing
// legacy-api-types.ts, which is on the source-size ratchet.
import { api } from "./client";
import type {
  OrgToolApprovalPolicy,
  SessionApprovalMode,
  ToolApprovalRule,
  ToolReviewedData,
} from "./schema-types";

/** Approval modes, from least to most human involvement. */
export type ApprovalModeName = "off" | "auto" | "normal" | "protective";

export const APPROVAL_MODES: readonly ApprovalModeName[] = ["off", "auto", "normal", "protective"];

/** What a rule does with a matching tool call. */
export type ToolRuleActionName = ToolApprovalRule["action"];

/** A person's own approval settings, stored as the `tool_approval` preference. */
export interface PersonalToolApproval {
  /** Mode the person's sessions start in; absent keeps each agent's own mode. */
  default_mode?: ApprovalModeName;
  /** Only `ask` and `never`: a personal rule may add caution, never remove it. */
  rules?: ToolApprovalRule[];
}

/** User-preference key holding a person's own approval settings. */
export const PERSONAL_APPROVAL_PREFERENCE_KEY = "tool_approval";

/** Audit event type of one auto review. */
export const AUTO_REVIEW_AUDIT_EVENT = "agent.approval.auto_reviewed";

declare module "./legacy-api-types" {
  interface Organization {
    /** Which approval modes people and agents may use, rules and reviewer notes. */
    tool_approval_policy?: OrgToolApprovalPolicy;
  }
  interface UpdateOrganizationRequest {
    /** Admin only; replaces the whole policy. */
    tool_approval_policy?: OrgToolApprovalPolicy;
  }
  interface EventTypeMap {
    /** The auto reviewer decided on a tool call. */
    "tool.reviewed": ToolReviewedData;
  }
}

export async function getSessionApprovalMode(sessionId: string): Promise<SessionApprovalMode> {
  const response = await api.get<SessionApprovalMode>(
    `/v1/platform/sessions/${encodeURIComponent(sessionId)}/approval-mode`,
  );
  return response.data;
}

/** `null` clears the switch so the session follows the defaults again. */
export async function setSessionApprovalMode(
  sessionId: string,
  mode: ApprovalModeName | null,
): Promise<SessionApprovalMode> {
  const response = await api.put<SessionApprovalMode>(
    `/v1/platform/sessions/${encodeURIComponent(sessionId)}/approval-mode`,
    { mode },
  );
  return response.data;
}
