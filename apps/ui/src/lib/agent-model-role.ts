// An agent either pins a model or follows an organization model role.
// The agent model picker holds the role as one more option value, so the
// draft keeps a single "model" field; requests translate it back.

import type { UpdateAgentRequest } from "@/lib/api/types";

/** Picker value for "Organization research model" (`model_role: "research"`). */
export const RESEARCH_MODEL_OPTION = "role:research";

/** The picker value for an agent's saved model choice. */
export function agentModelValue(agent: {
  default_model_id?: string | null;
  model_role?: string | null;
}): string {
  if (agent.model_role === "research") return RESEARCH_MODEL_OPTION;
  return agent.default_model_id || "";
}

/** The model fields of an update request for a picker value. */
export function agentModelRequest(
  value: string | undefined,
  initial: string,
): Pick<UpdateAgentRequest, "default_model_id" | "model_role"> {
  if (value === RESEARCH_MODEL_OPTION) return { model_role: "research" };
  // Leaving the research model stops following it; a picked model replaces it
  // server-side too.
  const leftRole = initial === RESEARCH_MODEL_OPTION;
  return {
    default_model_id: value || undefined,
    ...(leftRole && !value && { model_role: null }),
  };
}

/** The model to show for a picker value: the pinned one, or the org research model. */
export function agentModelId(
  value: string,
  organization: { effective_research_model_id?: string | null } | undefined,
): string {
  if (value === RESEARCH_MODEL_OPTION) return organization?.effective_research_model_id ?? "";
  return value;
}
