import { fireEvent, render, screen } from "@testing-library/react";
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

  it("keeps stale refs visible with an actionable remove control", () => {
    const onRemove = jest.fn();

    render(
      <SelectedCapabilityList
        selected={[{ ref: "plugin:plugin_stale", config: {} }]}
        getCapability={() => undefined}
        getDependents={() => []}
        onRemove={onRemove}
        onConfigChange={jest.fn()}
        onMoveUp={jest.fn()}
        onMoveDown={jest.fn()}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("Capability unavailable");
    expect(screen.getByRole("alert")).toHaveTextContent("plugin:plugin_stale");
    fireEvent.click(screen.getByRole("button", { name: /remove unavailable capability/i }));
    expect(onRemove).toHaveBeenCalledWith("plugin:plugin_stale");
  });
});
