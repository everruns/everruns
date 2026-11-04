import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { SavedChangesBar, UndoneBar } from "@/components/chat-mode/saved-changes-bar";
import type { SavedPageChange } from "@/components/chat-mode/page-change-review-controller";

const resource = { kind: "model", id: "model_1", label: "GPT-6.1 Sol" };
function receipt(overrides: Partial<SavedPageChange> = {}): SavedPageChange {
  return {
    resource,
    from: 1,
    revision: 2,
    stale: false,
    pending: false,
    fields: [
      { field: "display_name", from: "Sol", to: "Sol (fast)" },
      { field: "enabled", from: false, to: true },
      { field: "$secrets.api_key", from: "old-private", to: "new-private" },
    ],
    ...overrides,
  };
}
function setup(props: Partial<Parameters<typeof SavedChangesBar>[0]> = {}) {
  const handlers = { onUndo: jest.fn(), onDismiss: jest.fn(), onHistory: jest.fn() };
  render(<SavedChangesBar receipt={receipt()} dirty={false} {...handlers} {...props} />);
  return handlers;
}

it("stays collapsed to one line until Review is opened", () => {
  const { onUndo } = setup();
  expect(screen.getByRole("status")).toHaveTextContent("Saved 2 changes");
  expect(screen.queryByText("Sol (fast)")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  expect(onUndo).toHaveBeenCalledWith();
});

it("reviews each change with its own Undo and never shows secrets", () => {
  const { onUndo, onHistory } = setup();
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  const row = screen.getByRole("listitem", { name: "Change to display name" });
  expect(within(row).getByText("Sol")).toBeInTheDocument();
  expect(within(row).getByText("Sol (fast)")).toBeInTheDocument();
  fireEvent.click(within(row).getByRole("button", { name: "Undo display name" }));
  expect(onUndo).toHaveBeenCalledWith("display_name");
  expect(screen.queryByText(/private/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  expect(onHistory).toHaveBeenCalledWith(resource);
  expect(screen.getByRole("button", { name: "Undo all" })).toBeEnabled();
});

it("folds long values behind Show values", () => {
  setup({
    receipt: receipt({ fields: [{ field: "profile", from: { a: 1 }, to: { a: 2 } }] }),
  });
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  expect(screen.queryByText(/"a": 2/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Show values" }));
  expect(screen.getByText(/"a": 2/)).toBeInTheDocument();
});

it("blocks Undo while a draft is open and explains why", () => {
  setup({ dirty: true });
  expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  expect(screen.getByText(/Save or discard it before undoing/)).toBeInTheDocument();
});

it("points to History instead of Undo once the resource changed again", () => {
  setup({ receipt: receipt({ stale: true }) });
  expect(screen.getByRole("status")).toHaveTextContent("Changed again since your save");
  expect(screen.queryByRole("button", { name: "Undo" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "History" })).toBeInTheDocument();
});

it("dismisses the confirmation after a few seconds", () => {
  jest.useFakeTimers();
  const onDone = jest.fn();
  render(<UndoneBar count={2} resource={resource} onHistory={jest.fn()} onDone={onDone} />);
  expect(screen.getByRole("status")).toHaveTextContent("Undid 2 changes");
  act(() => jest.advanceTimersByTime(5_000));
  expect(onDone).toHaveBeenCalled();
  jest.useRealTimers();
});
