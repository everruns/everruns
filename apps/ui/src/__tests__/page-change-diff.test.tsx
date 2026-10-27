import { fireEvent, render, screen, within } from "@testing-library/react";
import { PageChangeDiff } from "@/components/chat-mode/page-change-diff";

it("renders the before and after values and selects only the requested field for Undo", () => {
  const undo = jest.fn();
  render(
    <PageChangeDiff
      fields={[
        { field: "name", from: "Northstar", to: "Northstar Review" },
        { field: "tags", from: ["old"], to: ["reviewed"] },
      ]}
      undo={undo}
    />,
  );
  const name = screen.getByRole("region", { name: "Change to name" });
  expect(within(name).getByText("Northstar")).toBeInTheDocument();
  expect(within(name).getByText("Northstar Review")).toBeInTheDocument();
  fireEvent.click(within(name).getByRole("button", { name: "Undo name" }));
  expect(undo).toHaveBeenCalledWith("name");
  expect(screen.getByText(/reviewed/)).toBeInTheDocument();
});

it("does not render secret history markers or their values", () => {
  render(
    <PageChangeDiff
      fields={[{ field: "$secrets.api_key", from: "old-private", to: "new-private" }]}
    />,
  );
  expect(screen.queryByText(/private/)).not.toBeInTheDocument();
  expect(screen.queryByText(/Review changes/)).not.toBeInTheDocument();
});

it("keeps the diff readable while Undo is blocked by a pending or stale state", () => {
  render(
    <PageChangeDiff
      fields={[{ field: "name", from: "Before", to: "After" }]}
      pending
      undo={jest.fn()}
    />,
  );
  expect(screen.getByRole("button", { name: "Undo name" })).toBeDisabled();
  expect(screen.getAllByText("After")).toHaveLength(2);
});
