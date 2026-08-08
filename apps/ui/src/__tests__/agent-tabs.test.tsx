import { agentEditTabItems, getAgentDetailTabItems } from "@/components/agents/agent-tabs";

function labels(items: ReturnType<typeof getAgentDetailTabItems>): React.ReactNode[] {
  return items.map((item) => item.label);
}

describe("agent tab definitions", () => {
  it("keeps the detail workflow order when versions are enabled", () => {
    expect(labels(getAgentDetailTabItems(true))).toEqual([
      "Overview",
      "Preview",
      "Credentials",
      "Triggers",
      "Versions",
      "Stats",
      "Integrate",
    ]);
  });

  it("removes only versions when its feature is disabled", () => {
    expect(labels(getAgentDetailTabItems(false))).toEqual([
      "Overview",
      "Preview",
      "Credentials",
      "Triggers",
      "Stats",
      "Integrate",
    ]);
  });

  it("reuses the shared preview definition in edit navigation", () => {
    expect(agentEditTabItems.map((item) => item.label)).toEqual(["Edit", "Preview"]);
  });
});
