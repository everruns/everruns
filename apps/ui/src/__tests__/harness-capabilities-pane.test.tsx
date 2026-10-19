import { render, screen } from "@testing-library/react";
import { HarnessCapabilitiesPane } from "@/components/harnesses/harness-capabilities-pane";
import type { HarnessDraft } from "@/components/harnesses/use-harness-draft";
import type { Capability } from "@/lib/api/types";

jest.mock("@/components/chat/streamdown-message", () => ({
  InlineStreamdownMessage: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

function capability(overrides: Partial<Capability> = {}): Capability {
  return {
    id: "skills",
    name: "Agent Skills",
    description:
      "Discover and activate skills from the session filesystem.\n\nSkills are instruction packages.\n\n> [!TIP]\n> Use list_skills.",
    status: "available",
    icon: "wand",
    feature_flag: "skills",
    ...overrides,
  };
}

function draft(refs: string[]): HarnessDraft {
  return {
    capabilities: refs.map((ref) => ({ ref, config: {} })),
  } as HarnessDraft;
}

describe("HarnessCapabilitiesPane", () => {
  it("shows catalog metadata and a details link for each resolved capability", () => {
    render(
      <HarnessCapabilitiesPane
        draft={draft(["skills", "infinity_context"])}
        editing={false}
        hasParent={false}
        allCapabilities={[
          capability(),
          capability({
            id: "infinity_context",
            name: "Infinity Context",
            description: "Trims older messages.",
            icon: "infinity",
            feature_flag: undefined,
          }),
        ]}
      />,
    );

    expect(screen.getByRole("link", { name: "Agent Skills" })).toHaveAttribute(
      "href",
      "/capabilities/skills",
    );
    expect(
      screen.getByText("Discover and activate skills from the session filesystem."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Use list_skills/)).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "More details about Agent Skills" })).toHaveAttribute(
      "href",
      "/capabilities/skills",
    );
    expect(screen.getByRole("link", { name: "Infinity Context" })).toHaveAttribute(
      "href",
      "/capabilities/infinity_context",
    );
  });

  it("shows the raw reference when the catalog has no metadata", () => {
    render(
      <HarnessCapabilitiesPane
        draft={draft(["skills"])}
        editing={false}
        hasParent={false}
        allCapabilities={[]}
      />,
    );

    expect(screen.getByText("skills")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /More details/ })).not.toBeInTheDocument();
  });
});
