import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { EntityCardTags, EntityCardCapabilities } from "@/components/ui/entity-card";
import { EntityIdentity } from "@/components/ui/entity-identity";

describe("EntityCard", () => {
  it("lets users reveal all tags without crowding the initial overview", () => {
    render(<EntityCardTags tags={["sre", "simulation", "training", "safety"]} />);
    expect(screen.getByText("training")).toBeInTheDocument();
    expect(screen.queryByText("safety")).not.toBeInTheDocument();
    const more = screen.getByRole("button", { name: "Show 1 more tags" });
    expect(more).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(more);
    expect(screen.getByText("safety")).toBeInTheDocument();
    const less = screen.getByRole("button", { name: "Show fewer tags" });
    expect(less).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(less);
    expect(screen.queryByText("safety")).not.toBeInTheDocument();
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
});

describe("EntityCardCapabilities", () => {
  it("reveals the full configuration on demand", () => {
    render(
      <EntityCardCapabilities>
        {["Files", "Time", "Web", "Shell", "Memory"].map((name) => (
          <span key={name}>{name}</span>
        ))}
      </EntityCardCapabilities>,
    );
    expect(screen.queryByText("Memory")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show 1 more capabilities" }));
    expect(screen.getByText("Memory")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show fewer capabilities" }));
    expect(screen.queryByText("Memory")).not.toBeInTheDocument();
  });
});
