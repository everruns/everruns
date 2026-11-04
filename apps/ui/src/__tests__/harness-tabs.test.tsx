import { resolveHarnessTab } from "@/components/harnesses/harness-tabs";
describe("harness tab definitions", () => {
  it.each([
    ["overview", "harness"],
    ["preview", "preview"],
    ["integrate", "integrate"],
    ["stats", "stats"],
    [null, "harness"],
  ])("resolves ?tab=%s to the %s tab", (param, tab) => {
    expect(resolveHarnessTab(param)).toBe(tab);
  });
});
