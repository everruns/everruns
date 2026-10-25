import { ChatModeContext } from "@/components/chat-mode/chat-mode-context";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { z } from "zod";
import { type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";

const schema = z.strictObject({ name: z.string(), description: z.string() });
let adapter: PageDraftAdapter;
const saved = jest.fn();
const mode: ChatModeState = {
  page: { route: "/observers/observer_1", label: "Observer" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (_context, value) => {
    adapter = value!;
    return () => {};
  },
};
function Form({
  enabled = true,
  baseline = "v1",
  id = "observer_1",
  formSchema = schema,
}: {
  enabled?: boolean;
  baseline?: string;
  id?: string;
  formSchema?: typeof schema;
}) {
  const [values, setValues] = useState({ name: "Existing", description: "Manual requirement" });
  const dirty = values.name !== "Existing" || values.description !== "Manual requirement";
  const chat = usePageFormDraft({
    enabled,
    entityType: "observer",
    entityId: id,
    label: "Observer",
    schema: formSchema,
    baseline,
    values,
    dirty,
    writable: true,
    apply: setValues,
  });
  return (
    <>
      <input
        aria-label="Name"
        value={values.name}
        onChange={(e) => setValues({ ...values, name: e.target.value })}
      />
      <button onClick={() => setValues({ name: "Existing", description: "Manual requirement" })}>
        Discard
      </button>
      <button disabled={chat.blocked} onClick={saved}>
        Save
      </button>
      {chat.notice}
    </>
  );
}
beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});

it("invalidates an old adapter immediately when the field schema changes", () => {
  const { rerender } = render(
    <PageChatProvider value={mode}>
      <Form />
    </PageChatProvider>,
  );
  const old = adapter;
  const before = old.read(old.describe().binding);
  const replacement = z.strictObject({ name: z.string().max(8), description: z.string() });
  rerender(
    <PageChatProvider value={mode}>
      <Form formSchema={replacement} />
    </PageChatProvider>,
  );
  expect(() => old.read(before.binding)).toThrow(/no longer available/);
  expect(adapter.describe().binding).not.toBe(before.binding);
});
beforeEach(() => saved.mockClear());

it("edits the existing draft, supports Undo, and saves only through the normal action", () => {
  render(
    <PageChatProvider value={mode}>
      <Form />
    </PageChatProvider>,
  );
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { name: "Chat name" });
  });
  expect(screen.getByLabelText("Name")).toHaveValue("Chat name");
  expect(saved).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Undo"));
  expect(screen.getByLabelText("Name")).toHaveValue("Existing");
  const again = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(again.binding, again.revision, { name: "Reviewed name" });
  });
  fireEvent.click(screen.getByText("Save"));
  expect(saved).toHaveBeenCalledTimes(1);
});

it("preserves later manual edits and clears old markers on Discard", () => {
  render(
    <PageChatProvider value={mode}>
      <Form />
    </PageChatProvider>,
  );
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { name: "Chat name" });
  });
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Manual name" } });
  fireEvent.click(screen.getByText("Undo"));
  expect(screen.getByLabelText("Name")).toHaveValue("Manual name");
  expect(screen.getByRole("alert")).toHaveTextContent("overwrite your edit");
  fireEvent.click(screen.getByText("Discard"));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Fresh edit" } });
  expect(screen.queryByText("Draft edited by Chat")).not.toBeInTheDocument();
});

it("keeps the draft but blocks Save and new Chat patches after a saved-version conflict", () => {
  const { rerender } = render(
    <PageChatProvider value={mode}>
      <Form />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Manual name" } });
  rerender(
    <PageChatProvider value={mode}>
      <Form baseline="v2" />
    </PageChatProvider>,
  );
  expect(screen.getByLabelText("Name")).toHaveValue("Manual name");
  expect(screen.getByText("Save")).toBeDisabled();
  expect(adapter.describe().writable).toBe(false);
});

it("invalidates the preceding form even when two records share the same route", () => {
  const { rerender } = render(
    <PageChatProvider value={mode}>
      <Form />
    </PageChatProvider>,
  );
  const old = adapter;
  const binding = old.describe().binding;
  rerender(
    <PageChatProvider value={mode}>
      <Form id="observer_2" />
    </PageChatProvider>,
  );
  expect(() => old.read(binding)).toThrow(/no longer available/);
  expect(adapter.describe().binding).not.toBe(binding);
});

it("preserves the existing feature-off Save behavior", () => {
  const { rerender } = render(<Form />);
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Manual name" } });
  rerender(<Form baseline="v2" />);
  expect(screen.getByText("Save")).not.toBeDisabled();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("keeps Undo when Chat replaces an unsaved manual edit with the saved value", () => {
  render(
    <PageChatProvider value={mode}>
      <Form />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Manual draft" } });
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { name: "Existing" });
  });
  expect(screen.getByLabelText("Name")).toHaveValue("Existing");
  expect(screen.getByText("Draft edited by Chat")).toBeInTheDocument();
  fireEvent.click(screen.getByText("Undo"));
  expect(screen.getByLabelText("Name")).toHaveValue("Manual draft");
  expect(saved).not.toHaveBeenCalled();
});

it("does not bind forms outside the right-page subtree", () => {
  const registerPage = jest.fn(() => () => {});
  render(
    <ChatModeContext.Provider value={{ ...mode, registerPage }}>
      <Form />
    </ChatModeContext.Provider>,
  );
  expect(registerPage).not.toHaveBeenCalled();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("invalidates a closed form even after reopening with identical values", () => {
  const view = (enabled: boolean) => (
    <PageChatProvider value={mode}>
      <Form enabled={enabled} />
    </PageChatProvider>
  );
  const { rerender } = render(view(true));
  const old = adapter;
  const before = old.read(old.describe().binding);
  rerender(view(false));
  expect(() => old.read(before.binding)).toThrow(/no longer available/);
  rerender(view(true));
  expect(adapter.describe().binding).not.toBe(before.binding);
  expect(() => old.read(before.binding)).toThrow(/no longer available/);
  expect(() => adapter.patch(before.binding, before.revision, { name: "Delayed edit" })).toThrow(
    /no longer the current page/,
  );
  expect(screen.getByLabelText("Name")).toHaveValue("Existing");
});
