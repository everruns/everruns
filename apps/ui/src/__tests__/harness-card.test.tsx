import { render, screen } from "@testing-library/react";
import { HarnessCard } from "@/components/harnesses/harness-card";
import type { Harness } from "@/lib/api/types";
import { resolveHarnessInheritance } from "@/lib/harness-inheritance";

jest.mock("@/components/chat/streamdown-message", () => ({
  InlineStreamdownMessage: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

function harness(overrides: Partial<Harness> = {}): Harness {
  return {
    id: "harness-child",
    name: "child",
    display_name: "Child",
    description: "Child harness",
    parent_harness_id: null,
    default_model_id: null,
    tags: [],
    capabilities: [],
    is_built_in: false,
    status: "active",
    created_at: "2026-08-07T00:00:00Z",
    updated_at: "2026-08-07T00:00:00Z",
    archived_at: null,
    deleted_at: null,
    ...overrides,
  };
}

describe("HarnessCard inheritance", () => {
  it("keeps root cards quiet", () => {
    const root = harness();

    render(<HarnessCard harness={root} inheritance={resolveHarnessInheritance(root, new Map())} />);

    expect(screen.queryByText("Inherits from")).not.toBeInTheDocument();
  });

  it("links the direct parent and keeps built-in status separate", () => {
    const parent = harness({ id: "harness-parent", name: "generic", display_name: "Generic" });
    const child = harness({ parent_harness_id: parent.id, is_built_in: true });
    const inheritance = resolveHarnessInheritance(child, new Map([[parent.id, parent]]));

    render(<HarnessCard harness={child} inheritance={inheritance} />);

    expect(screen.getByText("Inherits from")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Generic" })).toHaveAttribute(
      "href",
      "/harnesses/harness-parent",
    );
    expect(screen.getByText("Built-in")).toBeInTheDocument();
  });

  it("marks archived parents and handles unavailable parents without a link", () => {
    const archivedParent = harness({
      id: "harness-archived",
      display_name: "Archived Parent",
      status: "archived",
    });
    const archivedChild = harness({ parent_harness_id: archivedParent.id });
    const { rerender } = render(
      <HarnessCard
        harness={archivedChild}
        inheritance={resolveHarnessInheritance(
          archivedChild,
          new Map([[archivedParent.id, archivedParent]]),
        )}
      />,
    );
    expect(screen.getByText("(archived)")).toBeInTheDocument();

    const missingChild = harness({ parent_harness_id: "harness-missing" });
    rerender(
      <HarnessCard
        harness={missingChild}
        inheritance={resolveHarnessInheritance(missingChild, new Map())}
      />,
    );
    expect(screen.getByText("unavailable harness")).toHaveAttribute("title", "harness-missing");
    expect(screen.queryByRole("link", { name: "unavailable harness" })).not.toBeInTheDocument();
  });

  it("truncates long parent names and labels local capability chips", () => {
    const longName = "A parent harness with a deliberately very long display name";
    const parent = harness({ id: "harness-parent", display_name: longName });
    const child = harness({
      parent_harness_id: parent.id,
      capabilities: [{ ref: "platform", config: {} }],
    });

    render(
      <HarnessCard
        harness={child}
        inheritance={resolveHarnessInheritance(child, new Map([[parent.id, parent]]))}
      />,
    );

    expect(screen.getByRole("link", { name: longName })).toHaveClass("min-w-0", "truncate");
    expect(screen.getByText("Declared capabilities")).toBeInTheDocument();
    expect(screen.getByLabelText("Locally declared capabilities")).toBeInTheDocument();
  });
});
