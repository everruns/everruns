// Org costs: the price list, the costs summary, and the org's calendar
// timezone. Policy management (what used to be budgets) stays on
// `/v1/budgets`; the UI calls them usage policies.

import { api } from "./client";

/** Where an amount came from. `mixed` only appears on grouped lines. */
export type CostBasis = "host" | "rate" | "provider" | "list_price" | "unpriced" | "mixed";

export type CostMeter = "model.tokens" | "sandbox.time";

export interface CostRate {
  id: string;
  meter: CostMeter;
  match: Record<string, string>;
  unit_price_usd?: number;
  list_price_multiplier?: number;
  unit: string;
  effective_from: string;
  retired: boolean;
  reason?: string;
  created_at: string;
}

export interface CreateCostRateRequest {
  meter: CostMeter;
  match?: Record<string, string>;
  unit_price_usd?: number;
  list_price_multiplier?: number;
  effective_from?: string;
  reason?: string;
}

export interface CostSummaryLine {
  key?: string;
  label?: string;
  basis: CostBasis;
  cost_usd: number;
  unpriced_records: number;
  records: number;
  quantity: number;
}

export interface CostSummary {
  from: string;
  to: string;
  timezone: string;
  cost_usd: number;
  unpriced_records: number;
  by_basis: CostSummaryLine[];
  by_source: CostSummaryLine[];
  by_agent: CostSummaryLine[];
  by_user: CostSummaryLine[];
  by_started_by: CostSummaryLine[];
  by_day: CostSummaryLine[];
}

export interface AgentCost {
  agent_id: string;
  from: string;
  cost_usd: number;
  unpriced_records: number;
}

export interface CostSettings {
  timezone?: string | null;
}

export async function listCostRates(): Promise<CostRate[]> {
  return (await api.get<CostRate[]>("/v1/cost-rates")).data;
}

export async function createCostRate(input: CreateCostRateRequest): Promise<CostRate> {
  return (await api.post<CostRate>("/v1/cost-rates", input)).data;
}

export async function retireCostRate(rateId: string, reason?: string): Promise<CostRate> {
  return (await api.post<CostRate>(`/v1/cost-rates/${rateId}/retire`, { reason })).data;
}

export async function getCostSummary(params?: {
  from?: string;
  to?: string;
}): Promise<CostSummary> {
  const search = new URLSearchParams();
  if (params?.from) search.set("from", params.from);
  if (params?.to) search.set("to", params.to);
  const qs = search.toString();
  return (await api.get<CostSummary>(`/v1/costs/summary${qs ? `?${qs}` : ""}`)).data;
}

export async function getAgentCost(agentId: string): Promise<AgentCost> {
  return (await api.get<AgentCost>(`/v1/agents/${agentId}/costs`)).data;
}

export async function getCostSettings(): Promise<CostSettings> {
  return (await api.get<CostSettings>("/v1/costs/settings")).data;
}

export async function updateCostSettings(input: CostSettings): Promise<CostSettings> {
  return (await api.patch<CostSettings>("/v1/costs/settings", input)).data;
}

/** Let this session continue past a usage policy's "ask to continue" step. */
export async function continueSessionPastAsk(sessionId: string): Promise<void> {
  await api.post(`/v1/sessions/${sessionId}/resume`);
}

export type PolicyStepAction = "notify" | "ask" | "stop";

export interface PolicyStep {
  at_percent: number;
  action: PolicyStepAction;
}

/** What a usage policy does as spend grows. Stored on the budget row. */
export interface UsagePolicy {
  name?: string;
  steps?: PolicyStep[];
  source?: "all" | "managed" | "own_keys";
}

/** The steps a policy runs; without explicit steps, the long-standing defaults. */
export function effectiveSteps(
  policy: UsagePolicy | undefined,
  limit: number,
  softLimit?: number | null,
): PolicyStep[] {
  if (policy?.steps?.length) return [...policy.steps].sort((a, b) => a.at_percent - b.at_percent);
  const steps: PolicyStep[] = [{ at_percent: 80, action: "notify" }];
  if (softLimit != null && limit > 0) {
    steps.push({ at_percent: Math.min(100, (softLimit / limit) * 100), action: "ask" });
  }
  steps.push({ at_percent: 100, action: "stop" });
  return steps.sort((a, b) => a.at_percent - b.at_percent);
}
