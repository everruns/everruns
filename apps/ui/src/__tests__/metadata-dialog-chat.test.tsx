import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import { KnowledgeIndexFormDialog } from "@/components/knowledge-indexes/knowledge-index-form-dialog";
import type { Memory, KnowledgeIndex } from "@/lib/api/types";

jest.mock("@/hooks/use-user-connections", () => ({ useUserConnections: () => ({ data: [] }) }));
jest.mock("@/components/knowledge-indexes/embedding-model-picker", () => ({
  EmbeddingModelPicker: ({ onValidityChange }: { onValidityChange: (valid: boolean) => void }) => {
    const React = jest.requireActual("react");
    React.useEffect(() => onValidityChange(true), [onValidityChange]);
    return null;
  },
}));

beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});

function setup(kind: "memory" | "index") {
  let adapter: PageDraftAdapter;
  const onSubmit = jest.fn();
  const mode: ChatModeState = {
    page: { route: kind === "memory" ? "/memory" : "/knowledge-indexes", label: "Fixture" },
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
        {kind === "memory" ? (
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
        ) : (
          <KnowledgeIndexFormDialog
            mode="edit"
            open={open}
            writable={writable}
            index={
              {
                id: "index_fixture",
                name: "Northstar docs",
                description: "Fictional",
                source_type: "git",
                source_config: source,
                embedding_model_id: "model_fixture",
              } as unknown as KnowledgeIndex
            }
            onOpenChange={() => {}}
            onSubmit={onSubmit}
          />
        )}
      </PageChatProvider>
    );
  }
  return { view, onSubmit, adapter: () => adapter!, read: () => adapter!.read(adapter!.binding!) };
}

it.each(["memory", "index"] as const)(
  "preserves a manual source draft and blocks a concurrent saved source in %s",
  async (kind) => {
    const form = setup(kind);
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
  },
);

it.each(["memory", "index"] as const)(
  "refreshes clean %s controls and excludes source configuration from Chat reads",
  async (kind) => {
    const form = setup(kind);
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
  },
);
