import { isHarnessDeprecated } from "@/lib/harness-deprecation";

describe("harness deprecation", () => {
  it("uses managed metadata without treating custom tags as platform policy", () => {
    expect(isHarnessDeprecated({ is_built_in: true, tags: ["deprecated"] })).toBe(true);
    expect(isHarnessDeprecated({ is_built_in: false, tags: ["deprecated"] })).toBe(false);
    expect(isHarnessDeprecated({ is_built_in: true, tags: [] })).toBe(false);
  });
});
