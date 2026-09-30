import { getAgentTabItems, resolveAgentTab } from "@/components/agents/agent-tabs";
import { render } from "@testing-library/react";

function labels(items: ReturnType<typeof getAgentTabItems>): string[] {
  return items.map((item) => render(<>{item.label}</>).container.textContent ?? "");
}

// One tab row: MCP and Credentials became config-column sheets, Sessions got
// its own tab, and Versions moved to the header overflow menu.
describe("agent tab definitions", () => {
  it("renders the single tab row in order", () => {
    expect(labels(getAgentTabItems())).toEqual([
      "Agent",
      "Preview",
      "Integrations",
      "Stats",
      "Sessions",
    ]);
  });

  it("shows the session count on the Sessions tab", () => {
    expect(labels(getAgentTabItems(12)).at(-1)).toBe("Sessions12");
  });

  it.each([
    ["mcp", "agent", "mcp"],
    ["credentials", "agent", "credentials"],
    ["versions", "agent", "versions"],
    ["overview", "agent", null],
    ["integrations", "integrations", null],
    ["sessions", "sessions", null],
    [null, "agent", null],
  ])("resolves ?tab=%s to the %s tab with sheet %s", (param, tab, section) => {
    expect(resolveAgentTab(param)).toEqual({ tab, section });
  });
});
