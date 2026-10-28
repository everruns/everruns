import type { AgentPreviewResponse, Harness, ModelProfile } from "@/lib/api/types";
import { resolveHarnessInheritance } from "@/lib/harness-inheritance";

/** Agent overrides the nearest harness model, then the organization default. */
export function resolveEstimateModel(
  agentModelId: string,
  harnessId: string | undefined,
  harnesses: Harness[],
): { modelId: string; source: "Agent model" | "Harness model" | "Organization default" } | null {
  if (agentModelId) return { modelId: agentModelId, source: "Agent model" };
  if (harnessId) {
    const byId = new Map(harnesses.map((harness) => [harness.id, harness]));
    const harness = byId.get(harnessId);
    if (!harness) return null;
    const inheritance = resolveHarnessInheritance(harness, byId);
    if (inheritance.hasCycle || inheritance.missingParentId) return null;
    const modelId = [...inheritance.chain]
      .reverse()
      .find((item) => item.default_model_id)?.default_model_id;
    if (modelId) return { modelId, source: "Harness model" };
  }
  return { modelId: "", source: "Organization default" };
}

export interface CostWorkload {
  calls: number;
  messageTokens: number;
  outputTokens: number;
  toolResultTokens: number;
}

// Starting assumptions, not predictions about an agent's behavior. A task is
// one fresh conversation; all prior outputs/tool results are resent each call.
export const COST_WORKLOADS = [
  {
    id: "quick",
    name: "Quick answer",
    calls: 1,
    messageTokens: 500,
    outputTokens: 300,
    toolResultTokens: 0,
  },
  {
    id: "standard",
    name: "Multi-step task",
    calls: 5,
    messageTokens: 1000,
    outputTokens: 800,
    toolResultTokens: 1500,
  },
  {
    id: "deep",
    name: "Deep research",
    calls: 15,
    messageTokens: 2000,
    outputTokens: 1500,
    toolResultTokens: 3000,
  },
] as const;

/** Rough text sizing only. Non-ASCII text gets more allowance than English. */
export function estimateTextTokens(text: string): number {
  let units = 0;
  for (const character of text) units += character.codePointAt(0)! <= 127 ? 1 : 4;
  return Math.ceil(units / 4);
}

export function estimateDefinitionTokens(preview: AgentPreviewResponse): number {
  return (
    estimateTextTokens(preview.system_prompt) +
    (preview.tools.length ? estimateTextTokens(JSON.stringify(preview.tools)) : 0) +
    100
  );
}

export interface TaskCostEstimate {
  inputTokens: number;
  outputTokens: number;
  inputCost: number;
  outputCost: number;
  totalCost: number;
  peakContext: number;
  exceedsContext: boolean;
  tiered: boolean;
  outputCapped: boolean;
}

export function estimateTaskCost(
  profile: ModelProfile | undefined,
  definitionTokens: number,
  workload: CostWorkload,
  maxOutputTokens?: number,
): TaskCostEstimate | null {
  const cost = profile?.cost;
  const valid = (value: number) => Number.isFinite(value) && value >= 0;
  if (
    !cost ||
    ![
      cost.input,
      cost.output,
      definitionTokens,
      workload.calls,
      workload.messageTokens,
      workload.outputTokens,
      workload.toolResultTokens,
    ].every(valid) ||
    !Number.isInteger(workload.calls) ||
    workload.calls < 1 ||
    workload.calls > 200 ||
    (maxOutputTokens !== undefined && (!valid(maxOutputTokens) || maxOutputTokens < 1)) ||
    cost.cost_tiers?.some((tier) => ![tier.above_tokens, tier.input, tier.output].every(valid))
  )
    return null;

  const outputPerCall = Math.min(
    workload.outputTokens,
    maxOutputTokens ?? Infinity,
    profile?.limits?.output ?? Infinity,
  );
  const result: TaskCostEstimate = {
    inputTokens: 0,
    outputTokens: 0,
    inputCost: 0,
    outputCost: 0,
    totalCost: 0,
    peakContext: 0,
    exceedsContext: false,
    tiered: false,
    outputCapped: outputPerCall < workload.outputTokens,
  };
  for (let call = 0; call < workload.calls; call++) {
    const input =
      definitionTokens +
      workload.messageTokens +
      call * (outputPerCall + workload.toolResultTokens);
    // Match the runtime: tiers apply to each request's input, strictly above
    // the threshold. Aggregate task tokens must never select a request tier.
    const tier = cost.cost_tiers
      ?.filter((item) => input > item.above_tokens)
      .reduce<(typeof cost.cost_tiers)[number] | undefined>(
        (best, item) => (!best || item.above_tokens > best.above_tokens ? item : best),
        undefined,
      );
    result.inputTokens += input;
    result.outputTokens += outputPerCall;
    result.inputCost += (input * (tier?.input ?? cost.input)) / 1_000_000;
    result.outputCost += (outputPerCall * (tier?.output ?? cost.output)) / 1_000_000;
    result.peakContext = Math.max(result.peakContext, input + outputPerCall);
    result.exceedsContext ||=
      input > (profile?.limits?.input ?? Infinity) ||
      input + outputPerCall > (profile?.limits?.context ?? Infinity);
    result.tiered ||= !!tier;
  }
  result.totalCost = result.inputCost + result.outputCost;
  return result;
}

/** Sensitivity scenarios, deliberately not labelled a statistical confidence interval. */
export function scaleWorkload(workload: CostWorkload, scale: number): CostWorkload {
  return {
    ...workload,
    calls: Math.max(1, Math.ceil(workload.calls * scale)),
    outputTokens: Math.ceil(workload.outputTokens * scale),
    toolResultTokens: Math.ceil(workload.toolResultTokens * scale),
  };
}

export function formatEstimateUsd(value: number, locale = "en-US"): string {
  if (value > 0 && value < 0.0001) return `<${formatEstimateUsd(0.0001, locale)}`;
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: value < 1 ? 4 : 2,
  }).format(value);
}
