import { agentTabHref, getAgentTabItems, resolveAgentTab } from "@/components/agents/agent-tabs";
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
    ["mcp", "agent", "mcp", null],
    ["credentials", "agent", "credentials", null],
    ["versions", "agent", null, "history"],
    ["overview", "agent", null, null],
    ["integrations", "integrations", null, null],
    ["sessions", "sessions", null, null],
    [null, "agent", null, null],
  ])(
    "resolves ?tab=%s to the %s tab with sheet %s and record sheet %s",
    (param, tab, section, recordSheet) => {
      expect(resolveAgentTab(param)).toEqual({ tab, section, recordSheet });
    },
  );
});

describe("agent tab URL", () => {
  it("omits the tab param on the Agent tab and keeps other params", () => {
    expect(agentTabHref("agent-1", "agent", "tab=stats&mode=edit")).toBe(
      "/agents/agent-1?mode=edit",
    );
  });

  it("records any other tab and preserves existing params", () => {
    expect(agentTabHref("agent-1", "integrations", "mode=edit")).toBe(
      "/agents/agent-1?mode=edit&tab=integrations",
    );
  });

  it("replaces a legacy sheet param with the selected tab", () => {
    expect(agentTabHref("agent-1", "sessions", "tab=credentials")).toBe(
      "/agents/agent-1?tab=sessions",
    );
  });
});
