import { joinTags, normalizeTags } from "@/lib/tags";

describe("tags helpers", () => {
  it("returns an empty list for non-array values", () => {
    expect(normalizeTags(null)).toEqual([]);
    expect(normalizeTags(undefined)).toEqual([]);
    expect(normalizeTags("ops")).toEqual([]);
  });

  it("filters non-string and blank entries", () => {
    expect(normalizeTags([" ops ", "", 42, null, "alerts"])).toEqual(["ops", "alerts"]);
  });

  it("joins normalized tags for form display", () => {
    expect(joinTags([" ops ", "", "alerts"])).toBe("ops, alerts");
    expect(joinTags(null)).toBe("");
  });
});
