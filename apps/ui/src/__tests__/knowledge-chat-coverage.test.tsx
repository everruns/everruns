import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { CollectionForm } from "@/components/knowledge/collection-form";
import { IndexerForm } from "@/components/knowledge/indexer-form";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
import {
  PageViewRegistryContext,
  usePageViewRegistry,
} from "@/components/chat-mode/page-view-state";
import {
  useKnowledgeAccountRows,
  useKnowledgeCollectionRows,
  useKnowledgeSearchRows,
  useKnowledgeSourceRows,
} from "@/components/knowledge/use-knowledge-page-context";
import type { KnowledgeCollection, KnowledgeIndexer, KnowledgeSource } from "@/lib/api/knowledge";

let role = "owner";
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_fixture", role } }),
}));
jest.mock("@/hooks/use-providers", () => ({ useModels: () => ({ data: [] }) }));
jest.mock("@tanstack/react-query", () => ({ useQuery: () => ({ data: [] }) }));
jest.mock("@/lib/api/knowledge", () => ({ createKnowledgeIndexer: jest.fn() }));
jest.mock("@/lib/api/credential-bindings", () => ({
  createScopedCredential: jest.fn(),
  saveCredentialValue: jest.fn(),
}));

const collection: KnowledgeCollection = {
  id: "kn_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  name: "Northstar",
  description: "Support docs",
  indexing: "off",
  embedding_model_id: null,
  indexer_id: null,
  revision: 1,
  published_revision: 1,
  rules: {},
  status: "active",
  file_count: 3,
};
let adapter: PageDraftAdapter;
const mode: ChatModeState = {
  page: { route: `/knowledge/${collection.id}/settings`, label: "Knowledge" },
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
beforeEach(() => {
  role = "owner";
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
  jest.clearAllMocks();
});

it.each(["owner", "admin", "member"])(
  "honors %s access to every collection field and stages without saving",
  (viewer) => {
    role = viewer;
    const save = jest.fn();
    render(
      <PageChatProvider value={mode}>
        <CollectionForm initial={collection} onSave={save} />
      </PageChatProvider>,
    );
    const before = read();
    expect(before.values).toEqual({
      name: "Northstar",
      description: "Support docs",
      mode: "off",
      model: "",
      indexer: "",
      rules: JSON.stringify({ ingest_exclude: [], index_exclude: [], read_exclude: [] }, null, 2),
    });
    const patch = {
      name: "Help center",
      description: "Reviewed",
      mode: "hybrid",
      model: "model_fixture",
      indexer: "idx_fixture",
      rules: '{"index_exclude":["drafts/**"]}',
    };
    if (viewer === "member") {
      expect(before.writable).toBe(false);
      expect(() => adapter.patch(before.binding, before.revision, patch)).toThrow(/read.only/);
      expect(read().values).toEqual(before.values);
    } else {
      act(() => {
        adapter.patch(before.binding, before.revision, patch);
      });
      expect(read().values).toEqual(patch);
      expect(screen.getByText("Draft edited by Chat")).toBeVisible();
      expect(screen.getAllByText("Before")).toHaveLength(6);
      expect(screen.getAllByText("After")).toHaveLength(6);
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
      expect(read().values).toEqual(before.values);
    }
    expect(save).not.toHaveBeenCalled();
  },
);

it("follows clean collection refreshes, preserves dirty work, and blocks a stale Save until Discard", () => {
  const save = jest.fn();
  const tree = (initial: KnowledgeCollection) => (
    <PageChatProvider value={mode}>
      <CollectionForm initial={initial} onSave={save} />
    </PageChatProvider>
  );
  const view = render(tree(collection));
  view.rerender(tree({ ...collection, name: "Latest", revision: 2 }));
  expect(screen.getByLabelText("Name")).toHaveValue("Latest");
  fireEvent.change(screen.getByLabelText("Description"), { target: { value: "Keep my draft" } });
  const old = read();
  view.rerender(tree({ ...collection, name: "Concurrent", revision: 3 }));
  expect(screen.getByLabelText("Description")).toHaveValue("Keep my draft");
  expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled();
  expect(() => adapter.patch(old.binding, old.revision, { name: "Late" })).toThrow();
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(screen.getByLabelText("Name")).toHaveValue("Concurrent");
  expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
  expect(save).not.toHaveBeenCalled();
});

it("keeps built-in collection identity read-only and rejects mixed patches atomically", () => {
  render(
    <PageChatProvider value={mode}>
      <CollectionForm initial={{ ...collection, name: "everruns-docs" }} onSave={jest.fn()} />
    </PageChatProvider>,
  );
  const before = read();
  expect(() =>
    adapter.patch(before.binding, before.revision, { name: "Renamed", description: "Partial" }),
  ).toThrow(/read.only/);
  expect(read().values).toEqual(before.values);
});

it("recognizes saved rules despite server key ordering and follows a later field Undo", () => {
  const tree = (initial: KnowledgeCollection) => (
    <PageChatProvider value={mode}>
      <CollectionForm initial={initial} onSave={jest.fn()} />
    </PageChatProvider>
  );
  const view = render(tree(collection));
  fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Reviewed" } });
  // A native Save returns canonical JSON, rather than the textarea's key order.
  const saved = {
    ...collection,
    name: "Reviewed",
    revision: 2,
    rules: { index_exclude: [], ingest_exclude: [], read_exclude: [] },
  };
  view.rerender(tree(saved));
  expect(read().dirty).toBe(false);
  expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
  expect(screen.queryByText("The saved version changed")).not.toBeInTheDocument();
  view.rerender(tree({ ...saved, name: collection.name, revision: 3 }));
  expect(screen.getByLabelText("Name")).toHaveValue(collection.name);
  expect(read().dirty).toBe(false);
});

it("never reads or patches the new search account's write-only key", () => {
  const onSaved = jest.fn();
  render(
    <PageChatProvider value={mode}>
      <IndexerForm onSaved={onSaved} />
    </PageChatProvider>,
  );
  fireEvent.change(screen.getByLabelText("API key"), {
    target: { value: "fixture-write-only-secret" },
  });
  const before = read();
  expect(Object.keys(before.values)).toEqual(["name", "endpoint"]);
  expect(JSON.stringify(before)).not.toContain("fixture-write-only-secret");
  expect(() =>
    adapter.patch(before.binding, before.revision, { name: "Partial", key: "overwrite" }),
  ).toThrow(/unsupported/);
  expect(screen.getByLabelText("Account name")).toHaveValue("");
  act(() => {
    adapter.patch(before.binding, before.revision, { name: "Northstar search" });
  });
  expect(screen.getByLabelText("API key")).toHaveValue("fixture-write-only-secret");
  expect(onSaved).not.toHaveBeenCalled();
});

it.each(["ready", "loading", "error"])(
  "projects new Knowledge tabs in %s state without config, snippets or credentials",
  (state) => {
    const registry = renderHook(() => usePageViewRegistry("/knowledge"));
    function Tabs() {
      const status = { loading: state === "loading", error: state === "error" };
      useKnowledgeCollectionRows([collection], status);
      useKnowledgeAccountRows(
        [
          {
            id: "idx_1",
            name: "Search",
            status: "ready",
            endpoint: "https://user:fixture-secret@example.test",
            credential_binding_id: "fixture-secret",
          } satisfies KnowledgeIndexer,
        ],
        status,
      );
      useKnowledgeSourceRows(
        [
          {
            id: "src_1",
            kind: "git",
            root_path: "/docs",
            config: { url: "https://user:fixture-secret@example.test" },
            sync_status: "failed",
            last_error: "fixture-secret",
            last_synced_at: null,
          } satisfies KnowledgeSource,
        ],
        status,
      );
      useKnowledgeSearchRows(
        {
          results: [
            {
              id: "hit_1",
              collection_id: collection.id,
              file_id: "file_1",
              version_id: "v1",
              path: "/guide.md",
              title: "Guide",
              snippet: "fixture-secret",
              source_uri: "https://example.test?token=fixture-secret",
              location: { secret: "fixture-secret" },
            },
          ],
          warnings: ["fixture-secret"],
          truncated: true,
        },
        status,
      );
      return null;
    }
    const view = render(
      <PageViewRegistryContext.Provider value={registry.result.current.registry}>
        <Tabs />
      </PageViewRegistryContext.Provider>,
    );
    const data = registry.result.current.read("/knowledge");
    expect(data.sections).toHaveLength(5);
    expect(JSON.stringify(data)).not.toContain("fixture-secret");
    if (state === "ready") {
      expect(JSON.stringify(data)).toContain("Guide");
      expect(data.sections[4]).toMatchObject({ rows: [{ truncated: true, warning_count: 1 }] });
    } else
      data.sections.forEach((section) =>
        expect(section).toMatchObject({ rows: [], load_state: state }),
      );
    expect(registry.result.current.read("/agents").sections).toEqual([]);
    view.unmount();
    expect(registry.result.current.read("/knowledge").sections).toEqual([]);
  },
);
