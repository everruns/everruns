import { render, screen } from "@testing-library/react";
import { SelectedCapabilityList } from "@/components/agents/selected-capability-list";
import type { AgentCapabilityConfig, Capability } from "@/lib/api/types";

function capability(overrides: Partial<Capability> = {}): Capability {
  return {
    id: "legacy_capability",
    name: "Legacy Capability",
    description: "Capability with legacy config data",
    status: "available",
    config_schema: {
      type: "object",
      properties: {
        endpoint: { type: "string", title: "Endpoint" },
      },
    },
    ...overrides,
  };
}

describe("SelectedCapabilityList", () => {
  it("renders capabilities whose legacy config is missing", () => {
    const selected = [
      { ref: "legacy_capability", config: undefined },
    ] as unknown as AgentCapabilityConfig[];
    const cap = capability();

    render(
      <SelectedCapabilityList
        selected={selected}
        getCapability={(id) => (id === cap.id ? cap : undefined)}
        getDependents={() => []}
        onRemove={jest.fn()}
        onConfigChange={jest.fn()}
        onMoveUp={jest.fn()}
        onMoveDown={jest.fn()}
      />,
    );

    expect(screen.getByText("Legacy Capability")).toBeInTheDocument();
  });
});
