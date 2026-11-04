"use client";

// Queries shared by the Knowledge tabs. Every tab of a collection reads the same
// cached collection, so switching tabs never refetches or flashes empty.

import { useCallback } from "react";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import {
  getKnowledge,
  listKnowledge,
  listKnowledgeIndexers,
  type KnowledgeCollection,
} from "@/lib/api/knowledge";

export function useCanManageKnowledge() {
  const { currentOrg } = useOrg();
  return currentOrg?.role === "owner" || currentOrg?.role === "admin";
}

export function useKnowledgeCollections() {
  const { currentOrg } = useOrg();
  return useInfiniteQuery({
    queryKey: ["knowledge", currentOrg?.public_id],
    queryFn: ({ pageParam }) => listKnowledge(pageParam),
    initialPageParam: "",
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: !!currentOrg,
  });
}

export function useKnowledgeCollection(collectionId: string) {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["knowledge", currentOrg?.public_id, collectionId],
    queryFn: () => getKnowledge(collectionId),
    enabled: !!currentOrg,
    // Sync and indexing progress move on their own; poll while the page is open.
    refetchInterval: 5000,
  });
}

export function useKnowledgeIndexers() {
  const { currentOrg } = useOrg();
  return useQuery({
    queryKey: ["knowledge-indexers", currentOrg?.public_id],
    queryFn: listKnowledgeIndexers,
    enabled: !!currentOrg,
  });
}

/** Refetches collections, their files and search accounts after a change. */
export function useRefreshKnowledge() {
  const cache = useQueryClient();
  return useCallback(
    () =>
      Promise.all([
        cache.invalidateQueries({ queryKey: ["knowledge"] }),
        cache.invalidateQueries({ queryKey: ["knowledge-indexers"] }),
        cache.invalidateQueries({ queryKey: ["file-source"] }),
      ]),
    [cache],
  );
}

export function isBuiltinCollection(collection: KnowledgeCollection | undefined) {
  return collection?.name === "everruns-docs";
}

export function indexingLabel(mode: KnowledgeCollection["indexing"]) {
  return mode === "off" ? "Files only" : mode === "lexical" ? "Keyword search" : "Hybrid search";
}

/** Where a collection's files come from, in one line. */
export function sourceSummary(collection: KnowledgeCollection) {
  if (isBuiltinCollection(collection)) return "Everruns documentation";
  const parts = (collection.sources ?? []).map((source) =>
    source.kind === "managed"
      ? "Managed files"
      : source.kind === "builtin"
        ? "Everruns documentation"
        : `${source.kind === "github" ? "GitHub" : "Git"} ${source.config.repository ?? source.config.url ?? ""}`.trim(),
  );
  return parts.join(" · ") || "Managed files";
}
