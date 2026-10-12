import { ciFilter, conditionValues, splitList, withCondition } from "@/lib/github-ci-filter";

describe("GitHub CI filter", () => {
  it("builds nothing when the draft narrows nothing", () => {
    expect(ciFilter({ changesOnly: false })).toBeUndefined();
    expect(ciFilter({ changesOnly: false, branches: [" "], workflows: [] })).toBeUndefined();
  });

  it("replaces one condition and keeps the others", () => {
    const start = ciFilter({ changesOnly: true, branches: ["main"], workflows: ["CI"] });
    const next = withCondition(start, "github.ci.branch", ["main", " release ", "main"]);
    expect(conditionValues(next, "github.ci.branch")).toEqual(["main", "release"]);
    expect(conditionValues(next, "github.ci.workflow")).toEqual(["CI"]);
    expect(next.changes_only).toEqual({ path: "github.ci.outcome" });
    const cleared = withCondition(next, "github.ci.branch", []);
    expect(conditionValues(cleared, "github.ci.branch")).toEqual([]);
    expect(cleared.conditions).toHaveLength(1);
  });

  it("splits comma-separated fields", () => {
    expect(splitList(" main, release ,,")).toEqual(["main", "release"]);
  });
});
