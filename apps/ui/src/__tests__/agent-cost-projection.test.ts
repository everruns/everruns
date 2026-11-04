import type { ModelProfile } from "@/lib/api/types";
import { formatPlanningUsd, projectTaskMix, type PlanningTask } from "@/lib/agent-cost-projection";
const quick: PlanningTask = {
  name: "Answer",
  description: "A question",
  assumption: "One reply",
  calls: 1,
  messageTokens: 100,
  outputTokens: 100,
  toolResultTokens: 0,
  weight: 3,
};
const research = { ...quick, name: "Research", calls: 5, toolResultTokens: 1000, weight: 1 };
const profile = { cost: { input: 2, output: 10 } } as ModelProfile;
it("weights actual per-task prices rather than averaging token assumptions", () => {
  const a = projectTaskMix([quick], profile, 100)!;
  const b = projectTaskMix([research], profile, 100)!;
  const mix = projectTaskMix([quick, research], profile, 100)!;
  expect(mix.typical).toBeCloseTo(a.typical! * 0.75 + b.typical! * 0.25);
  expect(mix.rows.map((r) => r.share)).toEqual([0.75, 0.25]);
  expect(mix.low).toBeLessThan(mix.typical!);
  expect(mix.high).toBeGreaterThan(mix.typical!);
});
it("withholds invalid, unknown-price and impossible estimates", () => {
  expect(projectTaskMix([{ ...quick, weight: 0 }], profile, 100)).toBeNull();
  expect(projectTaskMix([{ ...quick, calls: 101 }], profile, 100)).toBeNull();
  expect(projectTaskMix([{ ...quick, outputTokens: NaN }], profile, 100)).toBeNull();
  expect(projectTaskMix([quick], undefined, 100)?.typical).toBeNull();
  expect(
    projectTaskMix([research], { ...profile, limits: { context: 100, output: 1000 } }, 100)
      ?.typical,
  ).toBeNull();
});
it("excludes disabled tasks without hiding an oversized selected task", () => {
  const constrained = { ...profile, limits: { context: 1000, output: 1000 } };
  expect(
    projectTaskMix([quick, { ...research, weight: 0 }], constrained, 100)?.typical,
  ).not.toBeNull();
  expect(projectTaskMix([quick, research], constrained, 100)?.typical).toBeNull();
});

it("rounds headlines without presenting tiny spend as free", () => {
  expect(formatPlanningUsd(0.001, "en-US")).toBe("<$0.01");
  expect(formatPlanningUsd(0, "en-US")).toBe("$0.00");
  expect(formatPlanningUsd(0.097, "en-US")).toBe("$0.10");
});
