import { agentEditTabItems, getAgentDetailTabItems } from "@/components/agents/agent-tabs";

function labels(items: ReturnType<typeof getAgentDetailTabItems>): React.ReactNode[] {
  return items.map((item) => item.label);
}

// "Triggers" and "Integrate" collapsed into one "Integrations" tab in
// EVE-1009: both described how an agent is reached and when it runs, and the
// snippet tab could only show generic URLs because a multi-endpoint agent has
// no single address.
describe("agent tab definitions", () => {
  it("keeps the detail workflow order when versions are enabled", () => {
    expect(labels(getAgentDetailTabItems(true))).toEqual([
      "Overview",
      "Preview",
      "Credentials",
      "Integrations",
      "Versions",
      "Stats",
    ]);
  });

  it("removes only versions when its feature is disabled", () => {
    expect(labels(getAgentDetailTabItems(false))).toEqual([
      "Overview",
      "Preview",
      "Credentials",
      "Integrations",
      "Stats",
    ]);
  });

  it("reuses the shared preview definition in edit navigation", () => {
    expect(agentEditTabItems.map((item) => item.label)).toEqual(["Edit", "Preview"]);
  });
});
