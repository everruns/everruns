import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { EntityCard } from "@/components/ui/entity-card";
import { EntityIdentity } from "@/components/ui/entity-identity";

describe("EntityCard", () => {
  it("keeps long titles shrinkable beside header actions", () => {
    const { container } = render(
      <EntityCard title="A very long entity name" headerActions={<span>active</span>}>
        Body
      </EntityCard>,
    );

    expect(screen.getByText("A very long entity name")).toHaveClass("block", "truncate");
    expect(container.querySelector('[data-slot="entity-card-heading"]')).toHaveClass(
      "min-w-0",
      "flex-1",
    );
  });
});

describe("EntityIdentity", () => {
  it("keeps a long ID out of the layout and exposes its full value accessibly", async () => {
    const value = "plugin:plugin_019fda100f037c008024046d6b3d74c0";

    render(<EntityIdentity value={value}>Resend</EntityIdentity>);

    expect(screen.queryByText(value)).not.toBeInTheDocument();
    expect(screen.getByText("Resend").closest('[data-slot="entity-identity"]')).toHaveClass(
      "min-w-0",
      "max-w-full",
    );
    expect(screen.getByText("Resend")).toHaveClass("truncate");
    const button = screen.getByRole("button", { name: `Copy ID: ${value}` });
    expect(button).toHaveClass("shrink-0");

    fireEvent.focus(button);
    expect(await screen.findByText(`Copy ID: ${value}`)).toBeInTheDocument();
  });

  it("copies the exact namespace-prefixed ID and announces feedback", async () => {
    jest.useFakeTimers();
    const value = "plugin:plugin_019fda100f037c008024046d6b3d74c0";
    const writeText = jest.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<EntityIdentity value={value}>Resend</EntityIdentity>);

    const button = screen.getByRole("button", { name: `Copy ID: ${value}` });
    act(() => button.focus());
    expect(button).toHaveFocus();
    await act(async () => fireEvent.click(button));

    expect(writeText).toHaveBeenCalledWith(value);
    expect(screen.getByRole("status")).toHaveTextContent("Copied");

    act(() => jest.runOnlyPendingTimers());
    await waitFor(() => expect(screen.getByRole("status")).toBeEmptyDOMElement());
    jest.useRealTimers();
  });

  it("wraps a page-heading label without exposing the long ID to the layout", () => {
    const value = "entity_019fda100f037c008024046d6b3d74c0";

    render(
      <EntityIdentity value={value} truncate={false}>
        A deliberately long readable entity heading for a narrow viewport
      </EntityIdentity>,
    );

    expect(screen.getByText(/deliberately long readable/)).toHaveClass("break-words", "min-w-0");
    expect(screen.queryByText(value)).not.toBeInTheDocument();
  });
});
