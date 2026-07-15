import { MAX_COMPARE_RUNS, parseCompareRunIds } from "@/lib/eval-run-comparison";

const ids = Array.from(
  { length: MAX_COMPARE_RUNS + 2 },
  (_, i) => `evalrun_${String(i + 1).padStart(32, "0")}`,
);

describe("parseCompareRunIds", () => {
  it("keeps only unique eval run IDs up to the comparison limit", () => {
    const parsed = parseCompareRunIds(
      [ids[0], "../../../auth/me", ids[0], "evalrun_nothex", ...ids.slice(1)].join(","),
    );

    expect(parsed.runIds).toEqual(ids.slice(0, MAX_COMPARE_RUNS));
    expect(parsed.ignoredCount).toBe(5);
    expect(parsed.exceededLimit).toBe(true);
  });

  it("ignores encoded path traversal values", () => {
    const parsed = parseCompareRunIds(`..%2F..%2F..%2Fauth%2Fme,${ids[0]}`);

    expect(parsed.runIds).toEqual([ids[0]]);
    expect(parsed.ignoredCount).toBe(1);
  });
});
