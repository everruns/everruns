"use client";

import { useChatPageRows } from "@/components/chat-mode/page-view-state";
import type {
  KnowledgeCollection,
  KnowledgeIndexer,
  KnowledgeResults,
  KnowledgeSource,
} from "@/lib/api/knowledge";

// Project the rows this UI actually displays. Never include source config,
// credential bindings, file bodies, search snippets or raw failure messages.
export function useKnowledgeCollectionRows(
  collections: readonly KnowledgeCollection[],
  state: { loading?: boolean; error?: boolean } = {},
) {
  useChatPageRows(
    "Knowledge collections",
    collections.map((c) => ({
      id: c.id,
      name: c.name,
      indexing: c.indexing,
      status: c.status,
      file_count: c.file_count,
      indexed_count: c.indexed_count,
    })),
    state,
  );
}

export function useKnowledgeAccountRows(
  accounts: readonly KnowledgeIndexer[],
  state: { loading?: boolean; error?: boolean } = {},
) {
  useChatPageRows(
    "Knowledge search accounts",
    accounts.map((a) => ({
      id: a.id,
      name: a.name,
      status: a.status,
    })),
    state,
  );
}

export function useKnowledgeSourceRows(
  sources: readonly KnowledgeSource[],
  state: { loading?: boolean; error?: boolean } = {},
) {
  useChatPageRows(
    "Knowledge sources",
    sources.map((s) => ({
      id: s.id,
      kind: s.kind,
      root_path: s.root_path,
      sync_status: s.sync_status,
      last_synced_at: s.last_synced_at,
      has_error: !!s.last_error,
    })),
    state,
  );
}

export function useKnowledgeSearchRows(
  results: KnowledgeResults | null,
  state: { loading?: boolean; error?: boolean } = {},
) {
  useChatPageRows(
    "Knowledge search results",
    (results?.results ?? []).map((r) => ({
      id: r.id,
      collection_id: r.collection_id,
      file_id: r.file_id,
      version_id: r.version_id,
      path: r.path,
      title: r.title,
    })),
    { ...state, enabled: results !== null },
  );
  useChatPageRows(
    "Knowledge search coverage",
    results
      ? [
          {
            truncated: results.truncated,
            warning_count: results.warnings.length,
          },
        ]
      : [],
    { ...state, enabled: results !== null },
  );
}
