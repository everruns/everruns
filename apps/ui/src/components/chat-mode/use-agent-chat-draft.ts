"use client";

import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { AgentDraft, AgentDraftFields } from "@/components/agents/use-agent-draft";
import type { FieldDiff } from "@/lib/api/types";
import { normalizeCommunication } from "@/lib/agent-communication";
import { usePageSavedChangeGuard } from "./page-change-review-provider";
import {
  agentDraftPatchSchema,
  agentDraftPatchJsonSchema,
  assertDraftSize,
  type AgentDraftPatch,
  type PageDraftAdapter,
  type PageDraftSnapshot,
} from "./page-draft";

export interface DraftChatChanges {
  fields: FieldDiff[];
  undo: (field?: string) => void;
  clear: () => void;
  error: string | null;
}

export function useAgentChatDraft({
  agentId,
  updatedAt,
  draft,
  writable,
  blocked,
  beginEdit,
}: {
  agentId: string;
  updatedAt?: string;
  draft: AgentDraft;
  writable: boolean;
  blocked: boolean;
  beginEdit: () => void;
}) {
  const binding = useMemo(() => crypto.randomUUID(), [agentId]);
  const savedGuard = usePageSavedChangeGuard({
    id: binding,
    entityType: "agent",
    entityId: agentId,
    dirty: draft.isDirty,
    baseline: updatedAt ?? agentId,
    enabled: true,
  });
  const savedBlocked =
    blocked || savedGuard.conflict || (draft.isDirty && (savedGuard.loading || !!savedGuard.error));
  const [marks, setMarks] = useState<FieldDiff[]>([]);
  const [error, setError] = useState<string | null>(null);
  const current = useRef({ agentId, updatedAt, draft, writable, blocked: savedBlocked, beginEdit });
  const version = useRef({ signature: "", revision: "" });
  const optimistic = useRef<PageDraftSnapshot<AgentDraftPatch> | null>(null);
  const mounted = useRef(false);
  useLayoutEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useLayoutEffect(() => {
    current.current = { agentId, updatedAt, draft, writable, blocked: savedBlocked, beginEdit };
    optimistic.current = null;
  });

  const adapter = useMemo<PageDraftAdapter<AgentDraftPatch>>(() => {
    function snapshot(): PageDraftSnapshot<AgentDraftPatch> {
      if (optimistic.current) return optimistic.current;
      const state = current.current;
      const values = {
        ...state.draft.fields,
        communication: normalizeCommunication(state.draft.fields.communication),
        capabilities: state.draft.capabilities,
        starters: state.draft.starters,
        initial_files: state.draft.files,
        network_access: state.draft.networkAccess ?? {},
        localizations: state.draft.localizations ?? {},
      };
      const signature = JSON.stringify([
        state.agentId,
        state.updatedAt,
        values,
        state.draft.sandboxPolicy,
        state.writable,
        state.blocked,
      ]);
      if (signature !== version.current.signature)
        version.current = { signature, revision: crypto.randomUUID() };
      return {
        binding,
        revision: version.current.revision,
        entity_id: state.agentId,
        entity_type: "agent",
        patch_schema: agentDraftPatchJsonSchema,
        dirty: state.draft.isDirty,
        writable: state.writable && !state.blocked && state.agentId === agentId,
        values,
        sandbox_policy: state.draft.sandboxPolicy,
      };
    }
    function bound(token: string) {
      if (!mounted.current || token !== binding || current.current.agentId !== agentId)
        throw new Error(
          "The Agent page has changed. Ask the user to send a message from the current page.",
        );
      const value = snapshot();
      assertDraftSize(value);
      return structuredClone(value);
    }
    function apply(changes: AgentDraftPatch, previous: PageDraftSnapshot<AgentDraftPatch>) {
      const state = current.current;
      const next = {
        ...previous,
        revision: crypto.randomUUID(),
        dirty: true,
        values: { ...previous.values, ...changes },
      };
      assertDraftSize(next);
      version.current = {
        signature: JSON.stringify([
          state.agentId,
          state.updatedAt,
          next.values,
          state.draft.sandboxPolicy,
          state.writable,
          state.blocked,
        ]),
        revision: next.revision,
      };
      // Reserve the new revision before scheduling React updates. A duplicate or
      // second patch in the same batch cannot reuse the preceding revision.
      optimistic.current = next;
      const { capabilities, starters, initial_files, network_access, localizations, ...fields } =
        changes;
      for (const [key, value] of Object.entries(fields))
        state.draft.setField(key as keyof AgentDraftFields, value);
      if (capabilities !== undefined) state.draft.setCapabilities(capabilities);
      if (starters !== undefined) state.draft.setStarters(starters);
      if (initial_files !== undefined) state.draft.setFiles(initial_files);
      if (network_access !== undefined) state.draft.setNetworkAccess(network_access);
      if (localizations !== undefined) state.draft.setLocalizations(localizations);
      state.beginEdit();
      return structuredClone(next);
    }
    return {
      binding,
      describe: () => {
        if (!mounted.current)
          return { binding, revision: version.current.revision, dirty: false, writable: false };
        const { revision, dirty, writable } = snapshot();
        return { binding, revision, dirty, writable };
      },
      read: bound,
      patch: (token, revision, input) => {
        const previous = bound(token);
        if (!previous.writable)
          throw new Error(
            "This Agent draft is read-only, saving, or has a saved-definition conflict.",
          );
        if (previous.revision !== revision)
          throw new Error(
            "The draft changed since you read it. Read it again and preserve the user's edits.",
          );
        assertDraftSize(input);
        const changes = agentDraftPatchSchema.parse(input);
        const fields = Object.entries(changes).filter(
          ([field, value]) =>
            JSON.stringify(previous.values[field as keyof AgentDraftPatch]) !==
            JSON.stringify(value),
        );
        if (!fields.length) return previous;
        const next = apply(changes, previous);
        setMarks((old) => {
          const updated = new Map(old.map((mark) => [mark.field, mark]));
          for (const [field, to] of fields) {
            const existing = updated.get(field);
            const preceding = previous.values[field as keyof AgentDraftPatch];
            // A manual edit between Chat patches establishes a new inverse.
            const from =
              existing && JSON.stringify(existing.to) === JSON.stringify(preceding)
                ? existing.from
                : preceding;
            if (JSON.stringify(from) === JSON.stringify(to)) updated.delete(field);
            else updated.set(field, { field, from, to });
          }
          return [...updated.values()];
        });
        setError(null);
        return next;
      },
    };
  }, [binding, agentId]);

  const clear = () => {
    setMarks([]);
    setError(null);
  };
  const undo = (field?: string) => {
    try {
      const snapshot = adapter.read(binding);
      const selected = marks.filter((mark) => !field || mark.field === field);
      if (!snapshot.writable) throw new Error("The draft cannot be changed right now.");
      if (
        selected.some(
          (mark) =>
            JSON.stringify(snapshot.values[mark.field as keyof AgentDraftPatch]) !==
            JSON.stringify(mark.to),
        )
      )
        throw new Error(
          "You edited a Chat-changed field. Undo would overwrite that edit; use the form to resolve it.",
        );
      if (!selected.length) return;
      adapter.patch(
        binding,
        snapshot.revision,
        Object.fromEntries(selected.map((mark) => [mark.field, mark.from])),
      );
      setMarks((old) => old.filter((mark) => !selected.some((item) => item.field === mark.field)));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not undo this draft change.");
    }
  };
  return { adapter, changes: { fields: marks, undo, clear, error } satisfies DraftChatChanges };
}
