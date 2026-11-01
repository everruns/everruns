import {
  deltaTone,
  experimentCost,
  formatPercent,
  formatPoints,
  passedEveryTrial,
  relativeChange,
  verdictDetail,
  verdictHeadline,
} from "@/components/evals/experiment-format";
import { experimentRequest } from "@/components/evals/run-experiment-dialog";
import type { EvalExperiment, RunSummary, VariantVerdict } from "@/lib/api/types";

const ui = (message: string, values?: Record<string, unknown>) =>
  values
    ? message.replace(/\{(\w+)\}/g, (token, key: string) =>
        key in values ? String(values[key]) : token,
      )
    : message;

function summary(overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    total: 6,
    passed: 3,
    failed: 3,
    errored: 0,
    pass_rate: 0.5,
    avg_score: 0.5,
    avg_turns: 2,
    avg_latency_ms: 1000,
    total_input_tokens: 0,
    total_output_tokens: 0,
    cases: 2,
    cases_passed_every_trial: 1,
    ...overrides,
  };
}

describe("experiment numbers", () => {
  it("signs points with a real minus", () => {
    expect(formatPoints(4.6)).toBe("+5");
    expect(formatPoints(-3.2)).toBe("−3");
    expect(formatPoints(0.2)).toBe("0");
  });

  it("formats rates and leaves missing ones blank", () => {
    expect(formatPercent(0.934)).toBe("93%");
    expect(formatPercent(null)).toBe("—");
  });

  it("computes pass^k from the case counts", () => {
    expect(passedEveryTrial(summary())).toBe(0.5);
    expect(passedEveryTrial(summary({ cases: 0 }))).toBeNull();
    expect(passedEveryTrial(undefined)).toBeNull();
  });

  it("compares cost relative to the baseline and calls a rise bad", () => {
    expect(relativeChange(0.035, 0.031)).toBe(13);
    expect(relativeChange(0.03, 0)).toBeNull();
    expect(deltaTone(13, false)).toBe("bad");
    expect(deltaTone(5, true)).toBe("good");
    expect(deltaTone(0, true)).toBe("neutral");
  });

  it("adds up the spend of finished variants only", () => {
    const experiment = {
      variants: [
        {
          label: "a",
          run_id: "r1",
          status: "completed",
          summary: summary({ total_cost_usd: 1.5 }),
        },
        { label: "b", run_id: "r2", status: "running" },
      ],
    } as unknown as EvalExperiment;
    expect(experimentCost(experiment)).toBe(1.5);
  });
});

describe("verdict wording", () => {
  const improvement: VariantVerdict = {
    label: "v15",
    paired_cases: 42,
    fixed: 4,
    broke: 1,
    verdict: "improvement",
    pass_rate_delta_points: 5,
    interval_low_points: 1,
    interval_high_points: 9,
  };

  it("says what changed and why it is not noise", () => {
    expect(verdictHeadline("improvement", ui)).toBe("Likely a real improvement.");
    expect(verdictDetail(improvement, ui)).toBe(
      "v15 fixed 4 cases and broke 1 case. The pass rate gain is outside run-to-run noise (95% interval +1 to +9 points).",
    );
  });

  it("explains a missing verdict instead of guessing", () => {
    expect(
      verdictDetail(
        { label: "luna", paired_cases: 2, fixed: 0, broke: 0, verdict: "not_enough_data" },
        ui,
      ),
    ).toBe("2 paired cases is too few to tell noise from a change; three or more are needed.");
  });
});

describe("run experiment form", () => {
  const draft = {
    evalId: "eval_1",
    variants: [
      { label: "baseline", model: "default" },
      { label: "sol", model: "model_sol" },
    ],
    trials: "3",
    maxCost: "10",
  };

  it("builds the request with only real model overrides", () => {
    expect(experimentRequest(draft)).toEqual({
      request: {
        variants: [{ label: "baseline" }, { label: "sol", model_override: "model_sol" }],
        trials: 3,
        max_cost_usd: 10,
      },
    });
  });

  it.each([
    [{ evalId: "" }, "Choose a dataset."],
    [
      {
        variants: [
          { label: "a", model: "default" },
          { label: " a ", model: "default" },
        ],
      },
      "Variant labels must differ.",
    ],
    [
      {
        variants: [
          { label: "", model: "default" },
          { label: "b", model: "default" },
        ],
      },
      "Every variant needs a label.",
    ],
    [{ trials: "0" }, "Trials are a whole number from 1 to 20."],
    [{ trials: "2.5" }, "Trials are a whole number from 1 to 20."],
    [{ maxCost: "-1" }, "The spending cap is a positive amount in USD."],
  ])("rejects %o", (patch, error) => {
    expect(experimentRequest({ ...draft, ...patch })).toEqual({ error });
  });

  it("leaves the cap out when empty", () => {
    const built = experimentRequest({ ...draft, maxCost: " " });
    expect(built).toEqual({
      request: expect.not.objectContaining({ max_cost_usd: expect.anything() }),
    });
  });
});
