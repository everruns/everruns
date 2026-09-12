import { Shield } from "lucide-react";
import { getHarnessIcon } from "@/lib/harness-icons";

describe("getHarnessIcon", () => {
  it("falls back to the neutral harness glyph when no icon is set", () => {
    expect(getHarnessIcon()).toBe(Shield);
    expect(getHarnessIcon(null)).toBe(Shield);
  });

  it("falls back for an icon name the UI does not know", () => {
    expect(getHarnessIcon("not-a-real-icon")).toBe(Shield);
  });

  it("resolves built-in harness icon names", () => {
    expect(getHarnessIcon("everruns")).not.toBe(Shield);
    expect(getHarnessIcon("box")).not.toBe(Shield);
    expect(getHarnessIcon("square-dashed")).not.toBe(Shield);
    expect(getHarnessIcon("bar-chart")).not.toBe(Shield);
    expect(getHarnessIcon("container")).not.toBe(Shield);
    expect(getHarnessIcon("terminal")).not.toBe(Shield);
    expect(getHarnessIcon("daytona")).not.toBe(Shield);
  });
});
