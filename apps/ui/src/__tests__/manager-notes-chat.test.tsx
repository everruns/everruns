import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ManagerNotesSheet } from "@/components/entity-actions/manager-notes-sheet";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";

const save = jest.fn();
let revision = 1;
let content = "Preserve the support SLA";
let loading = false;
jest.mock("@/hooks/use-change-history", () => ({
  useManagerContext: () => ({
    data: { content, revision },
    isLoading: loading,
    refetch: jest.fn(),
  }),
  useSetManagerContext: () => ({ mutateAsync: save, reset: jest.fn(), isPending: false }),
}));
jest.mock("@/components/ui/prompt-editor", () => ({
  MarkdownDisplay: ({ content }: { content: string }) => <p>{content}</p>,
}));
let adapter: PageDraftAdapter;
const mode: ChatModeState = {
  page: { route: "/agents/agent_fixture", label: "Agent" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (_, value) => {
    if (value) adapter = value;
    return () => {};
  },
};
const read = () => adapter.read(adapter.describe().binding);
const tree = (open = true) => (
  <PageChatProvider value={mode}>
    <ManagerNotesSheet
      entityRef="agent_fixture"
      entityName="Northstar"
      kind="agent"
      open={open}
      onOpenChange={() => {}}
    />
  </PageChatProvider>
);
beforeEach(() => {
  revision = 1;
  content = "Preserve the support SLA";
  loading = false;
  save.mockReset().mockResolvedValue(undefined);
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
});

it("targets the notes sheet, requires Edit, and stages and undoes notes before explicit Save", async () => {
  render(tree());
  expect(read()).toMatchObject({
    entity_type: "manager_notes",
    writable: false,
    values: { content },
  });
  const readonly = read();
  expect(() =>
    adapter.patch(readonly.binding, readonly.revision, { content: "Unauthorized edit" }),
  ).toThrow(/read.only/);
  fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  const before = read();
  act(() => {
    adapter.patch(before.binding, before.revision, { content: "Review the support SLA" });
  });
  expect(screen.getByRole("textbox", { name: "Manager notes" })).toHaveValue(
    "Review the support SLA",
  );
  expect(save).not.toHaveBeenCalled();
  expect(screen.getByText("Before")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  expect(screen.getByRole("textbox", { name: "Manager notes" })).toHaveValue(content);
  const current = read();
  act(() => {
    adapter.patch(current.binding, current.revision, { content: "Reviewed SLA" });
  });
  fireEvent.click(screen.getByRole("button", { name: "Save notes" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({ content: "Reviewed SLA", expectedRevision: 1, reason: "" }),
  );
});

it("keeps a draft and blocks Save when notes change remotely, and rejects a closed binding", () => {
  const view = render(tree());
  fireEvent.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Manager notes" }), {
    target: { value: "My draft" },
  });
  const old = read();
  revision = 2;
  content = "Concurrent notes";
  view.rerender(tree());
  expect(screen.getByRole("textbox", { name: "Manager notes" })).toHaveValue("My draft");
  expect(screen.getByRole("button", { name: "Save notes" })).toBeDisabled();
  expect(() => adapter.patch(old.binding, old.revision, { content: "Late edit" })).toThrow();
  expect(save).not.toHaveBeenCalled();
  const captured = adapter;
  view.rerender(tree(false));
  expect(() => captured.read(old.binding)).toThrow(/no longer available/);
});
