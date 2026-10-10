import { render, screen } from "@testing-library/react";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import type { Capability } from "@/lib/api/types";

jest.mock("@/components/chat/streamdown-message", () => ({
  InlineStreamdownMessage: () => null,
}));

function capability(overrides: Partial<Capability> = {}): Capability {
  return {
    id: "slack",
    name: "Slack",
    description: "Slack channel",
    status: "available",
    ...overrides,
  };
}

describe("CapabilitySelector", () => {
  it("does not announce unreleased capabilities under the selected list", () => {
    render(
      <CapabilitySelector
        capabilities={[
          capability(),
          capability({
            id: "research",
            name: "Research",
            description: "Deep research",
            status: "coming_soon",
          }),
        ]}
        selected={[{ ref: "slack", config: {} }]}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByText("Slack")).toBeInTheDocument();
    expect(screen.queryByText("More capabilities coming soon")).not.toBeInTheDocument();
  });
});
