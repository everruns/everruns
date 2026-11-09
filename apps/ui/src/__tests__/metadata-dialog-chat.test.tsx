import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import type { Memory } from "@/lib/api/types";

jest.mock("@/hooks/use-user-connections", () => ({ useUserConnections: () => ({ data: [] }) }));

beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});

function setup() {
  let adapter: PageDraftAdapter;
  const onSubmit = jest.fn();
  const mode: ChatModeState = {
    page: { route: "/memory", label: "Fixture" },
    pageContainer: { current: null },
    open: true,
    setOpen: () => {},
    actions: [],
    recordAction: () => {},
    registerPage: (_page, value) => {
      if (value) adapter = value;
      return () => {};
    },
  };
  function view(branch: string, open = true, writable = true) {
    const source = {
      provider: "git",
      url: "https://example.test/docs.git",
      branch,
      root_folder: "",
      sync_interval_secs: 0,
    };
    return (
      <PageChatProvider value={mode}>
        <MemoryFormDialog
          mode="edit"
          open={open}
          writable={writable}
          memory={
            {
              id: "memory_fixture",
              name: "Northstar docs",
              description: "Fictional",
              source,
            } as Memory
          }
          onOpenChange={() => {}}
          onSubmit={onSubmit}
        />
      </PageChatProvider>
    );
  }
  return { view, onSubmit, adapter: () => adapter!, read: () => adapter!.read(adapter!.binding!) };
}

it("preserves a manual source draft and blocks a concurrent saved source", async () => {
  const form = setup();
  const { rerender } = render(form.view("main"));
  fireEvent.change(screen.getByLabelText("Branch"), { target: { value: "manual-branch" } });
  rerender(form.view("server-branch"));
  expect(screen.getByLabelText("Branch")).toHaveValue("manual-branch");
  expect(screen.getByRole("button", { name: "Save Changes" })).toBeDisabled();
  const value = form.read();
  expect(value.writable).toBe(false);
  expect(() => form.adapter().patch(value.binding, value.revision, { name: "New name" })).toThrow(
    /read.only/,
  );
  expect(form.onSubmit).not.toHaveBeenCalled();
  rerender(form.view("server-branch", false));
  rerender(form.view("server-branch"));
  await waitFor(() => expect(screen.getByLabelText("Branch")).toHaveValue("server-branch"));
});

it("refreshes clean controls and excludes source configuration from Chat reads", async () => {
  const form = setup();
  const { rerender } = render(form.view("main"));
  rerender(form.view("latest"));
  await waitFor(() => expect(screen.getByLabelText("Branch")).toHaveValue("latest"));
  expect(form.read().values).toEqual({ name: "Northstar docs", description: "Fictional" });
  expect(JSON.stringify(form.read())).not.toContain("example.test");
  const value = form.read();
  act(() => {
    form.adapter().patch(value.binding, value.revision, { name: "Review docs" });
  });
  expect(screen.getByLabelText("Branch")).toHaveValue("latest");
  expect(form.onSubmit).not.toHaveBeenCalled();
  rerender(form.view("latest", true, false));
  expect(form.read().writable).toBe(false);
});
