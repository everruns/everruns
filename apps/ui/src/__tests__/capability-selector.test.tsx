import { fireEvent, render, screen, within } from "@testing-library/react";
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

  it("leads the Network category with Web search and keeps provider searches", () => {
    render(
      <CapabilitySelector
        capabilities={[
          capability({ id: "brave_search", name: "Brave Search", category: "Network" }),
          capability({ id: "duckduckgo", name: "DuckDuckGo", category: "Network" }),
          capability({ id: "web_search", name: "Web search", category: "Network" }),
        ]}
        selected={[]}
        onChange={jest.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    const dialog = screen.getByRole("dialog");
    const names = within(dialog)
      .getAllByRole("checkbox")
      .map((box) => box.closest("label")?.textContent ?? "");
    expect(names[0]).toContain("Web search");
    expect(names[0]).toContain("Recommended");
    expect(names.slice(1).join(" ")).toMatch(/Brave Search.*DuckDuckGo/);
  });
});
