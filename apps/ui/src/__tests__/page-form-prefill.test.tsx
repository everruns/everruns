import { useState } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { z } from "zod";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import { usePageFormPrefill } from "@/components/chat-mode/use-page-form-prefill";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";

const schema = z.strictObject({ name: z.string() });
let adapter: PageDraftAdapter;
const mode: ChatModeState = {
  page: { route: "/settings/mcp-catalog", label: "MCP" },
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
function Form({ name, id = "one", open = true }: { name: string; id?: string; open?: boolean }) {
  const [values, setValues] = useState({ name: "" });
  const saved = { name };
  usePageFormPrefill({ identity: id, open, values, saved, apply: setValues });
  const chat = usePageFormDraft({
    schema,
    entityType: "mcp_server",
    entityId: id,
    label: "MCP server",
    enabled: open,
    values,
    baseline: JSON.stringify(saved),
    dirty: values.name !== name,
    writable: true,
    apply: setValues,
  });
  return (
    <>
      <input
        aria-label="Name"
        value={values.name}
        onChange={(e) => setValues({ name: e.target.value })}
      />
      <button disabled={chat.blocked}>Save</button>
      <button onClick={() => setValues(saved)}>Discard</button>
      {chat.notice}
    </>
  );
}
beforeAll(() => {
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
});
it("refreshes clean fields, preserves dirty fields on unchanged and changed refetches, and recovers on Discard", () => {
  const tree = (name: string) => (
    <PageChatProvider value={mode}>
      <Form name={name} />
    </PageChatProvider>
  );
  const { rerender } = render(tree("Saved"));
  expect(screen.getByLabelText("Name")).toHaveValue("Saved");
  rerender(tree("Fresh"));
  expect(screen.getByLabelText("Name")).toHaveValue("Fresh");
  expect(screen.getByText("Save")).toBeEnabled();
  const snapshot = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(snapshot.binding, snapshot.revision, { name: "Draft" });
  });
  rerender(tree("Fresh"));
  expect(screen.getByLabelText("Name")).toHaveValue("Draft");
  rerender(tree("Concurrent"));
  expect(screen.getByLabelText("Name")).toHaveValue("Draft");
  expect(screen.getByText("Save")).toBeDisabled();
  expect(() => adapter.patch(snapshot.binding, snapshot.revision, { name: "Late" })).toThrow();
  fireEvent.click(screen.getByText("Discard"));
  expect(screen.getByLabelText("Name")).toHaveValue("Concurrent");
  expect(screen.getByText("Save")).toBeEnabled();
});
it("reinitializes on reopen or a different resource without reviving an old binding", () => {
  const tree = (id: string, open: boolean) => (
    <PageChatProvider value={mode}>
      <Form id={id} name={id} open={open} />
    </PageChatProvider>
  );
  const { rerender } = render(tree("one", true));
  const old = adapter;
  const snapshot = old.read(old.describe().binding);
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Unsaved" } });
  rerender(tree("one", false));
  rerender(tree("one", true));
  expect(screen.getByLabelText("Name")).toHaveValue("one");
  expect(() => old.read(snapshot.binding)).toThrow();
  rerender(tree("two", true));
  expect(screen.getByLabelText("Name")).toHaveValue("two");
});
