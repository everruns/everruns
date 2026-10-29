import {
  COST_WORKLOADS,
  estimateDefinitionTokens,
  estimateTaskCost,
  estimateTextTokens,
  formatEstimateUsd,
  resolveEstimateModel,
  scaleWorkload,
} from "@/lib/agent-cost-estimate";
import type { Harness, ModelProfile } from "@/lib/api/types";

const profile = { cost: { input: 2, output: 10 } } as ModelProfile;
const workload = { calls: 3, messageTokens: 500, outputTokens: 200, toolResultTokens: 800 };

describe("agent cost estimation", () => {
  it("applies cache buckets per tier and an explicit model retry allowance", () => {
    const cached = {
      ...profile,
      cost: {
        input: 2,
        output: 10,
        cache_read: 0.2,
        cache_write: 2.5,
        cost_tiers: [{ above_tokens: 2000, input: 4, output: 20, cache_read: 0.4, cache_write: 5 }],
      },
    };
    const result = estimateTaskCost(cached, 1000, workload, undefined, {
      cacheReadPercent: 50,
      cacheWritePercent: 25,
      retryPercent: 10,
    })!;
    expect(result.inputCost).toBeCloseTo(
      ((1500 * (0.5 + 0.1 + 0.625) + 6000 * (1 + 0.2 + 1.25)) / 1e6) * 1.1,
    );
    expect(result.outputCost).toBeCloseTo(((200 * (10 + 20 + 20)) / 1e6) * 1.1);
    expect(
      estimateTaskCost(cached, 1000, workload, undefined, {
        cacheReadPercent: 80,
        cacheWritePercent: 25,
        retryPercent: 0,
      }),
    ).toBeNull();
    expect(
      estimateTaskCost(profile, 1000, workload, undefined, {
        cacheReadPercent: 50,
        cacheWritePercent: 25,
        retryPercent: 0,
      })!.totalCost,
    ).toBeCloseTo(0.021);
  });
  it("bills repeated definitions and growing conversation context on every call", () => {
    const result = estimateTaskCost(profile, 1000, workload)!;
    // Inputs: 1500, 2500, 3500. Outputs: 3 * 200.
    expect(result.inputTokens).toBe(7500);
    expect(result.outputTokens).toBe(600);
    expect(result.inputCost).toBeCloseTo(0.015);
    expect(result.outputCost).toBeCloseTo(0.006);
    expect(result.totalCost).toBeCloseTo(0.021);
    expect(result.peakContext).toBe(3700);
  });

  it("selects the highest applicable tier per request, strictly above its threshold", () => {
    const tiered = {
      ...profile,
      cost: {
        input: 2,
        output: 10,
        cost_tiers: [
          { above_tokens: 3000, input: 8, output: 40 },
          { above_tokens: 1500, input: 4, output: 20 },
        ],
      },
    };
    const result = estimateTaskCost(tiered, 1000, workload)!;
    expect(result.inputCost).toBeCloseTo((1500 * 2 + 2500 * 4 + 3500 * 8) / 1e6);
    expect(result.outputCost).toBeCloseTo((200 * (10 + 20 + 40)) / 1e6);
    expect(result.tiered).toBe(true);
  });

  it("does not apply a long-context tier to aggregate task tokens", () => {
    const result = estimateTaskCost(
      {
        ...profile,
        cost: { ...profile.cost!, cost_tiers: [{ above_tokens: 5000, input: 100, output: 100 }] },
      },
      1000,
      workload,
    )!;
    expect(result.totalCost).toBeCloseTo(0.021);
    expect(result.tiered).toBe(false);
  });

  it("caps generated output and subsequent context at the tighter agent/model limit", () => {
    const limited = { ...profile, limits: { context: 10000, output: 150 } };
    const result = estimateTaskCost(limited, 1000, workload, 100)!;
    expect(result.outputTokens).toBe(300);
    expect(result.inputTokens).toBe(7200);
    expect(result.outputCapped).toBe(true);
    expect(estimateTaskCost(limited, 1000, workload, 500)!.outputTokens).toBe(450);
  });

  it("flags context and independent input limits rather than inventing compaction savings", () => {
    expect(
      estimateTaskCost({ ...profile, limits: { context: 3600, output: 1000 } }, 1000, workload)!
        .exceedsContext,
    ).toBe(true);
    expect(
      estimateTaskCost(
        { ...profile, limits: { context: 5000, input: 3000, output: 1000 } },
        1000,
        workload,
      )!.exceedsContext,
    ).toBe(true);
    expect(
      estimateTaskCost({ ...profile, limits: { context: 3700, output: 1000 } }, 1000, workload)!
        .exceedsContext,
    ).toBe(false);
  });

  it("distinguishes unknown prices from genuinely zero prices", () => {
    expect(estimateTaskCost(undefined, 1000, workload)).toBeNull();
    expect(estimateTaskCost({} as ModelProfile, 1000, workload)).toBeNull();
    expect(
      estimateTaskCost({ ...profile, cost: { input: 0, output: 0 } }, 1000, workload)!.totalCost,
    ).toBe(0);
    expect(
      estimateTaskCost({ ...profile, cost: { input: NaN, output: 1 } }, 1000, workload),
    ).toBeNull();
  });

  it.each([0, -1, 1.5, Infinity, NaN, 201])("rejects invalid call count %s", (calls) => {
    expect(estimateTaskCost(profile, 1000, { ...workload, calls })).toBeNull();
  });

  it("includes assembled tool definitions and sizes non-ASCII text explicitly", () => {
    expect(estimateTextTokens("abcd")).toBe(1);
    expect(estimateTextTokens("你好世界")).toBe(4);
    expect(estimateDefinitionTokens({ system_prompt: "abcd", tools: [] })).toBe(101);
    expect(
      estimateDefinitionTokens({
        system_prompt: "abcd",
        tools: [{ type: "client_side", name: "search", description: "Search", parameters: {} }],
      }),
    ).toBeGreaterThan(101);
  });

  it("scales workload assumptions before calculating nonlinear cost", () => {
    const base = COST_WORKLOADS[1];
    const low = scaleWorkload(base, 0.5);
    const high = scaleWorkload(base, 2);
    expect(low.calls).toBe(3);
    expect(high.messageTokens).toBe(base.messageTokens);
    expect(estimateTaskCost(profile, 1000, high)!.totalCost).toBeGreaterThan(
      estimateTaskCost(profile, 1000, base)!.totalCost * 2,
    );
    expect(scaleWorkload(COST_WORKLOADS[0], 0.5).calls).toBe(1);
  });

  it("does not round small nonzero costs to free", () => {
    expect(formatEstimateUsd(0)).toBe("$0.00");
    expect(formatEstimateUsd(0.000001)).toBe("<$0.0001");
    expect(formatEstimateUsd(0.0123)).toBe("$0.0123");
  });
});

describe("estimate model resolution", () => {
  const parent = {
    id: "parent",
    default_model_id: "parent-model",
    parent_harness_id: null,
    status: "active",
  } as Harness;
  const child = {
    id: "child",
    default_model_id: null,
    parent_harness_id: "parent",
    status: "active",
  } as Harness;
  it("uses agent, nearest harness, then organization defaults", () => {
    expect(resolveEstimateModel("agent-model", "child", [])?.modelId).toBe("agent-model");
    expect(resolveEstimateModel("", "child", [parent, child])?.modelId).toBe("parent-model");
    expect(
      resolveEstimateModel("", "child", [parent, { ...child, default_model_id: "child-model" }])
        ?.modelId,
    ).toBe("child-model");
    expect(resolveEstimateModel("", "child", [{ ...child, parent_harness_id: null }])?.source).toBe(
      "Organization default",
    );
  });
  it("does not silently fall back when a harness chain is unavailable or cyclic", () => {
    expect(resolveEstimateModel("", "child", [])).toBeNull();
    expect(resolveEstimateModel("", "child", [child])).toBeNull();
    expect(
      resolveEstimateModel("", "child", [child, { ...parent, parent_harness_id: "child" }]),
    ).toBeNull();
  });
});
