import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Suspense } from "react";
import KnowledgeIndexDetailPage from "@/app/(main)/knowledge-indexes/[indexId]/page";
import type { KnowledgeIndex, KnowledgeIndexDocument } from "@/lib/api/types";

const mockUseKnowledgeIndex = jest.fn();
const mockUseKnowledgeIndexDocuments = jest.fn();
const mockUseUpdateKnowledgeIndex = jest.fn();
const mockUseSyncKnowledgeIndex = jest.fn();
const mockUseArchiveKnowledgeIndex = jest.fn();
const mockUseUserConnections = jest.fn();
const mockUseModels = jest.fn();

jest.mock("@/hooks", () => ({
  useKnowledgeIndex: (...args: unknown[]) => mockUseKnowledgeIndex(...args),
  useKnowledgeIndexDocuments: (...args: unknown[]) => mockUseKnowledgeIndexDocuments(...args),
  useUpdateKnowledgeIndex: () => mockUseUpdateKnowledgeIndex(),
  useSyncKnowledgeIndex: () => mockUseSyncKnowledgeIndex(),
  useArchiveKnowledgeIndex: () => mockUseArchiveKnowledgeIndex(),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useUserConnections: () => mockUseUserConnections(),
}));

jest.mock("@/hooks/use-providers", () => ({
  useModels: () => mockUseModels(),
}));

const index: KnowledgeIndex = {
  id: "kidx_019dfb261a407c6085dcdd602402c3f7",
  name: "Product Docs",
  description: "Synced product documentation",
  source_type: "github",
  source_config: {
    provider: "github",
    repository: "everruns/everruns",
    branch: "main",
    root_folder: "docs",
  },
  embedding_model_id: "model_001",
  vector_dim: 1536,
  document_count: 1,
  status: "active",
  sync_status: "synced",
  last_synced_at: "2026-05-06T03:37:51.552849Z",
  last_sync_error: null,
  created_at: "2026-05-06T02:37:51.552849Z",
  updated_at: "2026-05-06T02:37:51.552849Z",
  archived_at: null,
  deleted_at: null,
};

const documents: KnowledgeIndexDocument[] = [
  {
    id: "kidoc_019dfb261a407c6085dcdd602402c3f7",
    index_id: index.id,
    source_uri: "github://everruns/everruns@main/docs/readme.md",
    title: "Readme",
    mime_type: "text/markdown",
    content_hash: "abc",
    size_bytes: 1024,
    chunk_count: 4,
    last_seen_at: "2026-05-06T03:37:51.552849Z",
    created_at: "2026-05-06T02:37:51.552849Z",
    updated_at: "2026-05-06T02:37:51.552849Z",
  },
];

async function renderWithSuspense(indexId = index.id) {
  const paramsPromise = Promise.resolve({ indexId });
  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <KnowledgeIndexDetailPage params={paramsPromise} />
      </Suspense>,
    );
    await paramsPromise;
  });
}

describe("KnowledgeIndexDetailPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseKnowledgeIndex.mockReturnValue({ data: index, isLoading: false, error: null });
    mockUseKnowledgeIndexDocuments.mockReturnValue({
      data: documents,
      isLoading: false,
      error: null,
    });
    mockUseUpdateKnowledgeIndex.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseSyncKnowledgeIndex.mockReturnValue({ mutate: jest.fn(), isPending: false });
    mockUseArchiveKnowledgeIndex.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseUserConnections.mockReturnValue({
      data: [{ provider: "github", connected_at: "2026-05-01T00:00:00Z" }],
      isLoading: false,
      error: null,
    });
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_001",
          display_name: "text-embedding-3-small",
          provider_name: "OpenAI",
          provider_id: "provider_001",
          provider_type: "openai_completions",
          enabled: true,
          healthy: true,
          capabilities: ["embeddings"],
        },
      ],
      isLoading: false,
      error: null,
    });
  });

  it("renders index metadata, source, and documents", async () => {
    await renderWithSuspense();

    expect(screen.getByRole("heading", { level: 1, name: "Product Docs" })).toBeInTheDocument();
    expect(screen.getAllByText("Synced product documentation").length).toBeGreaterThan(0);
    expect(screen.getByText("everruns/everruns")).toBeInTheDocument();
    expect(screen.getAllByText("model_001").length).toBeGreaterThan(0);
    expect(screen.getByText("Readme")).toBeInTheDocument();
    expect(screen.getByText(/docs\/readme\.md/)).toBeInTheDocument();
    expect(screen.getAllByText("4").length).toBeGreaterThan(0);
    expect(
      screen
        .getAllByRole("link", { name: /Knowledge Indexes/i })
        .some((l) => l.getAttribute("href") === "/knowledge-indexes"),
    ).toBe(true);
  });

  it("triggers a manual sync", async () => {
    const mutate = jest.fn();
    mockUseSyncKnowledgeIndex.mockReturnValue({ mutate, isPending: false });
    await renderWithSuspense();

    fireEvent.click(screen.getAllByRole("button", { name: /Sync now/i })[0]);
    expect(mutate).toHaveBeenCalledWith(index.id);
  });

  it("shows the last sync error when present", async () => {
    mockUseKnowledgeIndex.mockReturnValue({
      data: { ...index, sync_status: "failed", last_sync_error: "auth failed" },
      isLoading: false,
      error: null,
    });
    await renderWithSuspense();
    expect(screen.getByText(/auth failed/)).toBeInTheDocument();
  });

  it("surfaces an invalid configured model and requires repair before retry", async () => {
    mockUseModels.mockReturnValue({ data: [], isLoading: false });
    await renderWithSuspense();

    expect(screen.getAllByText("Embedding model unavailable").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button", { name: /Sync now/i })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByText(/No embedding models are configured/i)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Configure an embedding model" })).toHaveAttribute(
      "href",
      "/models",
    );
    expect(screen.getByRole("button", { name: "Save Changes" })).toBeDisabled();
  });

  it("renders an empty documents state", async () => {
    mockUseKnowledgeIndexDocuments.mockReturnValue({ data: [], isLoading: false, error: null });
    await renderWithSuspense();
    expect(screen.getByText(/No documents indexed yet/i)).toBeInTheDocument();
  });

  it("archives the index after confirmation", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseArchiveKnowledgeIndex.mockReturnValue({ mutateAsync, isPending: false });
    await renderWithSuspense();

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledWith(index.id));
  });

  it("renders not found state", async () => {
    mockUseKnowledgeIndex.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("404"),
    });
    await renderWithSuspense("kidx_missing");
    expect(screen.getByText("Knowledge index not found")).toBeInTheDocument();
    expect(screen.getByText("kidx_missing")).toBeInTheDocument();
  });

  it.each([
    [
      "not synced yet",
      { sync_status: "idle", last_synced_at: null, document_count: 0 },
      "Not synced yet",
      "Start initial sync",
    ],
    ["queued", { sync_status: "pending" }, "Sync queued", null],
    ["syncing", { sync_status: "syncing" }, "Syncing", null],
    ["synced empty", { document_count: 0 }, "Synced — no documents", "Sync again"],
    ["stale", { updated_at: "2026-05-06T04:37:51.552849Z" }, "Sync out of date", "Sync changes"],
    [
      "failed",
      { sync_status: "failed", last_sync_error: "Repository access was denied" },
      "Sync failed",
      "Retry sync",
    ],
  ])("shows the %s diagnostic and action", async (_name, overrides, label, action) => {
    mockUseKnowledgeIndex.mockReturnValue({
      data: { ...index, ...overrides },
      isLoading: false,
      error: null,
    });
    await renderWithSuspense();

    expect(
      screen.getAllByRole("status", { name: `Index status: ${label}` }).length,
    ).toBeGreaterThan(0);
    if (action) expect(screen.getAllByRole("button", { name: action }).length).toBeGreaterThan(0);
  });

  it("blocks sync for a generation-only embedding model", async () => {
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_001",
          display_name: "Claude Opus 4.7 (1M)",
          provider_name: "Anthropic",
          provider_id: "provider_anthropic",
          provider_type: "anthropic",
          enabled: true,
          healthy: true,
          capabilities: ["text"],
        },
      ],
      isLoading: false,
      error: null,
    });
    await renderWithSuspense();

    expect(
      screen.getAllByRole("status", { name: "Index status: Embedding model invalid" }).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText(/does not support embeddings/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configure embedding model" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Sync now/i })).not.toBeInTheDocument();
  });
});
