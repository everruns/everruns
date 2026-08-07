import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useSyncKnowledgeIndex } from "@/hooks/use-knowledge-indexes";
import type { KnowledgeIndex } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";

const mockSyncKnowledgeIndexNow = jest.fn();

jest.mock("@/lib/api/knowledge-indexes", () => ({
  syncKnowledgeIndexNow: (...args: unknown[]) => mockSyncKnowledgeIndexNow(...args),
}));

const failedIndex: KnowledgeIndex = {
  id: "kidx_019dfb261a407c6085dcdd602402c3f7",
  name: "Product Docs",
  description: null,
  source_type: "github",
  source_config: { repository: "everruns/bashkit" },
  embedding_model_id: "model_001",
  vector_dim: null,
  status: "active",
  sync_status: "failed",
  document_count: 0,
  last_synced_at: null,
  last_sync_error: "missing credentials",
  created_at: "2026-05-06T02:37:51.552849Z",
  updated_at: "2026-05-06T02:37:51.552849Z",
  archived_at: null,
  deleted_at: null,
};

describe("useSyncKnowledgeIndex", () => {
  it("updates only the detail cache and preserves cached documents", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const pendingIndex = {
      ...failedIndex,
      sync_status: "pending" as const,
      last_sync_error: null,
    };
    const documents = [{ id: "document_001", chunk_count: 2 }];
    queryClient.setQueryData(queryKeys.knowledgeIndexes.detail(failedIndex.id), failedIndex);
    queryClient.setQueryData(queryKeys.knowledgeIndexes.documents(failedIndex.id), documents);
    mockSyncKnowledgeIndexNow.mockResolvedValue(pendingIndex);
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useSyncKnowledgeIndex(), { wrapper });
    await act(async () => {
      await result.current.mutateAsync(failedIndex.id);
    });

    expect(queryClient.getQueryData(queryKeys.knowledgeIndexes.detail(failedIndex.id))).toEqual(
      pendingIndex,
    );
    expect(queryClient.getQueryData(queryKeys.knowledgeIndexes.documents(failedIndex.id))).toEqual(
      documents,
    );
  });
});
