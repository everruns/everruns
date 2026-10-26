import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { z } from "zod";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import { usePageSettingsDraft } from "@/components/chat-mode/use-page-settings-draft";

const schema = z.strictObject({ name: z.string(), enabled: z.boolean() });
let adapter: PageDraftAdapter;
const save = jest.fn();
const mode: ChatModeState = {
  page: { route: "/settings/example", label: "Settings" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (_, value) => {
    adapter = value!;
    return () => {};
  },
};
function Settings({ name = "Saved", writable = true }: { name?: string; writable?: boolean }) {
  const draft = usePageSettingsDraft({
    values: { name, enabled: true },
    schema,
    entityType: "settings",
    entityId: "org_1",
    label: "Settings",
    writable,
    save,
  });
  return (
    <>
      <input
        aria-label="Name"
        disabled={draft.pending}
        value={draft.values.name}
        onChange={(event) => draft.setValues({ ...draft.values, name: event.target.value })}
      />
      {draft.controls}
    </>
  );
}
beforeEach(() => {
  save.mockReset().mockResolvedValue(undefined);
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});

it("stages mouse and Chat edits without writes, and saves only the reviewed values", async () => {
  render(
    <PageChatProvider value={mode}>
      <Settings />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Manual" } });
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { enabled: false });
  });
  expect(adapter.read(before.binding).values).toEqual({ name: "Manual", enabled: false });
  expect(save).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ name: "Manual", enabled: false }));
});

it("preserves a dirty draft and blocks Save after a remote update until Discard", () => {
  const { rerender } = render(
    <PageChatProvider value={mode}>
      <Settings />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Draft" } });
  rerender(
    <PageChatProvider value={mode}>
      <Settings name="Concurrent" />
    </PageChatProvider>,
  );
  expect(screen.getByLabelText("Name")).toHaveValue("Draft");
  expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByLabelText("Name")).toHaveValue("Concurrent");
  expect(save).not.toHaveBeenCalled();
});

it("refuses patches while saving or read-only", async () => {
  let finish!: (value: unknown) => void;
  save.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const { rerender } = render(
    <PageChatProvider value={mode}>
      <Settings />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Draft" } });
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  const before = adapter.read(adapter.describe().binding);
  expect(() => adapter.patch(before.binding, before.revision, { name: "Late" })).toThrow(
    /read-only/,
  );
  expect(screen.getByLabelText("Name")).toBeDisabled();
  await act(async () => {
    finish(undefined);
  });
  rerender(
    <PageChatProvider value={mode}>
      <Settings writable={false} />
    </PageChatProvider>,
  );
  const readonly = adapter.read(adapter.describe().binding);
  expect(() => adapter.patch(readonly.binding, readonly.revision, { name: "Denied" })).toThrow(
    /read-only/,
  );
});

it("does not add save controls or expose a writable adapter with Adoption off", () => {
  render(
    <PageChatProvider value={null}>
      <Settings />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Manual" } });
  expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument();
  expect(save).not.toHaveBeenCalled();
});

it("retains the reviewed draft after a save failure and lets the person retry", async () => {
  save.mockRejectedValueOnce(new Error("Connection unavailable"));
  render(
    <PageChatProvider value={mode}>
      <Settings />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Keep me" } });
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Connection unavailable");
  expect(screen.getByLabelText("Name")).toHaveValue("Keep me");
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(save).toHaveBeenLastCalledWith({ name: "Keep me", enabled: true });
});
