import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { ConfirmationProvider, useConfirmation } from "@/components/ui/confirmation-provider";

it("keeps a failed action open, prevents repeat submissions, and supports retry", async () => {
  let reject!: () => void;
  const action = jest
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise((_, fail) => {
          reject = () => fail(new Error("private backend detail"));
        }),
    )
    .mockResolvedValueOnce(undefined);
  function Example() {
    const [open, setOpen] = useState(true);
    return (
      <ConfirmDialog
        open={open}
        onOpenChange={setOpen}
        title="Archive memory"
        description="Existing sessions retain their bindings."
        confirmLabel="Archive"
        onConfirm={action}
      />
    );
  }
  render(<Example />);
  const button = await screen.findByRole("button", { name: "Archive" });
  fireEvent.click(button);
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(action).toHaveBeenCalledTimes(1);
  reject();
  expect(await screen.findByRole("alert")).not.toHaveTextContent("private backend detail");
  fireEvent.click(button);
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(action).toHaveBeenCalledTimes(2);
});

it("cancels an unanswered operation when its route or organization unmounts", async () => {
  const result = jest.fn();
  function Consumer() {
    const confirm = useConfirmation();
    return <button onClick={async () => result(await confirm("Delete file?"))}>Delete</button>;
  }
  const view = render(
    <ConfirmationProvider>
      <Consumer />
    </ConfirmationProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(await screen.findByRole("dialog")).toBeInTheDocument();
  view.unmount();
  await waitFor(() => expect(result).toHaveBeenCalledWith(false));
});

it("cancels on a scope change while preserving the page's unsaved draft", async () => {
  const result = jest.fn();
  function Consumer() {
    const confirm = useConfirmation();
    const [draft, setDraft] = useState("");
    return (
      <>
        <input
          aria-label="Draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        <button onClick={async () => result(await confirm("Delete file?"))}>Delete</button>
      </>
    );
  }
  const view = render(
    <ConfirmationProvider scope="org-a:page">
      <Consumer />
    </ConfirmationProvider>,
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Draft" }), {
    target: { value: "Unsaved work" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(await screen.findByRole("dialog")).toBeInTheDocument();
  view.rerender(
    <ConfirmationProvider scope="org-a:page?section=files">
      <Consumer />
    </ConfirmationProvider>,
  );
  await waitFor(() => expect(result).toHaveBeenCalledWith(false));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Draft" })).toHaveValue("Unsaved work");
  view.rerender(
    <ConfirmationProvider scope="org-a:page">
      <Consumer />
    </ConfirmationProvider>,
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
