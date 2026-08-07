import { render, screen } from "@testing-library/react";
import { EntityCard, EntityCardIdentifier } from "@/components/ui/entity-card";

describe("EntityCard", () => {
  it("keeps long titles shrinkable beside header actions", () => {
    const { container } = render(
      <EntityCard title="A very long entity name" headerActions={<span>active</span>}>
        Body
      </EntityCard>,
    );

    expect(screen.getByText("A very long entity name")).toHaveClass("truncate");
    expect(container.querySelector('[data-slot="entity-card-heading"]')).toHaveClass(
      "min-w-0",
      "flex-1",
    );
  });
});

describe("EntityCardIdentifier", () => {
  it("truncates a long identifier while keeping its copy action visible", () => {
    const value = "identity_019fda100f037c008024046d6b3d74c0";

    render(<EntityCardIdentifier value={value} />);

    expect(screen.getByText(value)).toHaveClass("min-w-0", "truncate");
    expect(screen.getByTitle("Copy ID")).toHaveClass("shrink-0");
  });
});
