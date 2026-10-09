import { HarnessDomainIcon } from "@/components/icons/facet-icons";
import { getHarnessIcon } from "@/lib/harness-icons";

describe("getHarnessIcon", () => {
  it("falls back to the neutral harness glyph when no icon is set", () => {
    expect(getHarnessIcon()).toBe(HarnessDomainIcon);
    expect(getHarnessIcon(null)).toBe(HarnessDomainIcon);
  });

  it("falls back for an icon name the UI does not know", () => {
    expect(getHarnessIcon("not-a-real-icon")).toBe(HarnessDomainIcon);
  });

  it("resolves built-in harness icon names", () => {
    expect(getHarnessIcon("everruns")).not.toBe(HarnessDomainIcon);
    expect(getHarnessIcon("box")).not.toBe(HarnessDomainIcon);
    expect(getHarnessIcon("square-dashed")).not.toBe(HarnessDomainIcon);
    expect(getHarnessIcon("bar-chart")).not.toBe(HarnessDomainIcon);
    expect(getHarnessIcon("container")).not.toBe(HarnessDomainIcon);
    expect(getHarnessIcon("terminal")).not.toBe(HarnessDomainIcon);
    expect(getHarnessIcon("daytona")).not.toBe(HarnessDomainIcon);
  });
});
