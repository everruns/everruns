"use client";

import { useCallback, useEffect, useRef, useState, useContext, useSyncExternalStore } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { PageChangeReviewContext } from "./page-change-review-provider";
import { useChatMode } from "./chat-mode-context";
import { useOrg } from "@/providers/org-provider";
import {
  diffEntityRevisions,
  listEntityHistory,
  restoreEntityRevision,
} from "@/lib/api/change-history";
import { queryKeys } from "@/lib/query-keys";
import type { FieldDiff } from "@/lib/api/types";

export interface ChatChanges {
  canUndo?: boolean;
  shared?: boolean;
  fields: FieldDiff[];
  undo: (field?: string) => Promise<void>;
  dismiss: () => void;
  pending: boolean;
  error: string | null;
}

const noSubscribe = () => () => {};
const noSnapshot = () => 0;
export function useAgentChatChanges(agentId: string): ChatChanges {
  const controller = useContext(PageChangeReviewContext);
  useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.snapshot ?? noSnapshot,
    noSnapshot,
  );
  const legacy = useLegacyAgentChatChanges(agentId, !controller);
  if (!controller) return legacy;
  const resource = { kind: "agent", id: agentId, label: "Agent" };
  const receipt = controller
    .receipts()
    .find((item) => item.resource.kind === "agent" && item.resource.id === agentId);
  return {
    shared: true,
    fields: receipt?.fields ?? [],
    pending: receipt?.pending ?? false,
    canUndo: !!receipt && !receipt.stale && !controller.hasDirtyDraft(resource),
    error: receipt?.error ?? null,
    undo: (field) => controller.undo(resource, field),
    dismiss: () => controller.dismiss(resource),
  };
}
function useLegacyAgentChatChanges(agentId: string, enabled: boolean): ChatChanges {
  const mode = useChatMode();
  const { currentOrg } = useOrg();
  const queryClient = useQueryClient();
  const org = currentOrg?.public_id;
  const [fields, setFields] = useState<FieldDiff[]>([]);
  const [pending, setPending] = useState(false);
  const [loadingDiff, setLoadingDiff] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seen = useRef<number | null>(null);
  const base = useRef<number | null>(null);
  const ownUndos = useRef(new Set<number>());
  const dismissed = useRef(new Set<string>());
  const history = useQuery({
    queryKey: ["chat-mode-history", org, agentId],
    queryFn: () => listEntityHistory(agentId),
    enabled: enabled && !!mode && !!org,
    refetchInterval: enabled && mode?.open ? 1500 : false,
    refetchOnWindowFocus: !!mode,
  });
  const latest = history.data?.entries.find((entry) => entry.revision != null);
  const revision = latest?.revision ?? null;

  useEffect(() => {
    if (revision == null || !latest) return;
    if (seen.current == null) {
      seen.current = revision;
      return;
    }
    if (seen.current === revision) return;
    const previous = seen.current;
    seen.current = revision;
    void queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) });
    void queryClient.invalidateQueries({ queryKey: queryKeys.agents.all });
    if (ownUndos.current.delete(revision)) return;
    if (latest.via_session_id !== mode?.threadId || revision <= 1) {
      base.current = null;
      setFields([]);
      return;
    }
    // Only combine an uninterrupted run of this Chat's saves. A human edit
    // starts a fresh baseline, so undo never rewinds somebody else's work.
    const between = history.data?.entries.filter(
      (entry) => entry.revision != null && entry.revision > previous,
    );
    const interrupted = between?.some((entry) => entry.via_session_id !== mode?.threadId);
    if (interrupted) base.current = null;
    if (base.current == null) {
      const contiguous = [];
      for (const entry of between ?? []) {
        if (entry.via_session_id !== mode?.threadId) break;
        if (entry.revision != null) contiguous.push(entry.revision);
      }
      base.current = interrupted ? Math.min(...contiguous) - 1 : previous;
    }
    if (base.current < 1) return;
    dismissed.current.clear();
    const from = base.current;
    setLoadingDiff(true);
    diffEntityRevisions(agentId, from, revision)
      .then((diff) => {
        if (seen.current === revision && base.current === from)
          setFields(diff.filter((field) => !field.field.startsWith("$secrets")));
      })
      .catch((cause) => {
        if (seen.current === revision)
          setError(cause instanceof Error ? cause.message : "Could not load Chat changes.");
      })
      .finally(() => {
        if (seen.current === revision) setLoadingDiff(false);
      });
  }, [agentId, history.data, latest, mode?.threadId, queryClient, revision]);

  const dismiss = useCallback(() => {
    setFields([]);
    base.current = null;
    setError(null);
  }, []);
  const undo = async (field?: string) => {
    if (pending || loadingDiff || revision == null || base.current == null || !mode) return;
    const selected = fields
      .filter((diff) => (!field || diff.field === field) && !dismissed.current.has(diff.field))
      .map((diff) => diff.field);
    if (!selected.length) return;
    setPending(true);
    setError(null);
    try {
      ownUndos.current.add(revision + 1);
      const result = await restoreEntityRevision(
        agentId,
        base.current,
        field ? `Undo Chat change to ${field}` : "Undo Chat changes on the Agent page",
        { expected_revision: revision, fields: selected },
      );
      for (const name of selected) dismissed.current.add(name);
      setFields((current) => current.filter((diff) => !selected.includes(diff.field)));
      if (result.warnings.length) setError(result.warnings.join(" "));
      mode.recordAction(field ? `Undid Chat’s change to ${field}` : "Undid all Chat changes");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.agents.all }),
        history.refetch(),
      ]);
    } catch (cause) {
      ownUndos.current.delete(revision + 1);
      setError(cause instanceof Error ? cause.message : "Could not undo Chat changes.");
    } finally {
      setPending(false);
    }
  };
  return {
    fields,
    undo,
    dismiss,
    pending: pending || loadingDiff,
    error:
      error ?? (history.error ? "Could not follow agent changes. Try reopening this page." : null),
  };
}
