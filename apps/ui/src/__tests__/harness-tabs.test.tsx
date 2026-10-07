import { getHarnessTabItems, resolveHarnessTab } from "@/components/harnesses/harness-tabs";
import { render } from "@testing-library/react";

function labels(items: ReturnType<typeof getHarnessTabItems>): string[] {
  return items.map((item) => render(<>{item.label}</>).container.textContent ?? "");
}

describe("harness tab definitions", () => {
  it("renders the single tab row in order", () => {
    expect(labels(getHarnessTabItems())).toEqual(["Harness", "Preview", "Integrate", "Stats"]);
  });

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
