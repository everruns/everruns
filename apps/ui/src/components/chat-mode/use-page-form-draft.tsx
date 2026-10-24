"use client";

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { usePageChatMode } from "./chat-mode-context";
import { createFormDraftController, type FormDraftState } from "./form-draft-controller";
import type { FieldDiff } from "@/lib/api/types";

export const draftText = z.string().max(256 * 1024);

export function usePageFormDraft<T extends Record<string, unknown>>(
  options: FormDraftState<T> & {
    entityId?: string;
    entityType: string;
    label: string;
    schema: z.ZodType<T>;
    enabled?: boolean;
    formId?: string;
    priority?: number;
  },
) {
  const mode = usePageChatMode();
  const adopted = !!mode;
  const entityId = options.entityId ?? "new";
  const entityType = options.entityType;
  const route = mode?.page.route ?? "";
  const scope = `${route}:${options.entityType}:${options.entityId ?? "new"}:${options.formId ?? "main"}`;
  const schema = options.schema;
  const enabled = options.enabled ?? true;
  const identity = useMemo(() => crypto.randomUUID(), [scope, schema, adopted, enabled]);
  const [changes, setChanges] = useState<{ scope: string; baseline: string; fields: FieldDiff[] }>({
    scope,
    baseline: options.baseline,
    fields: [],
  });
  const [error, setError] = useState<string | null>(null);
  const appliedValues = useRef<string | null>(null);
  const [observedDirty, setObservedDirty] = useState(options.dirty);
  const [editingBaseline, setEditingBaseline] = useState(options.baseline);
  if (!options.dirty && editingBaseline !== options.baseline) setEditingBaseline(options.baseline);
  const blocked = adopted && options.dirty && editingBaseline !== options.baseline;
  if (observedDirty !== options.dirty) {
    setObservedDirty(options.dirty);
    if (!options.dirty && appliedValues.current !== JSON.stringify(options.values)) {
      setChanges({ scope, baseline: options.baseline, fields: [] });
      setError(null);
    }
  }
  const current = useRef({ ...options, scope, blocked, identity });
  const mounted = useRef(false);
  useLayoutEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useLayoutEffect(() => {
    current.current = { ...options, scope, blocked, identity };
  });
  const adapter = useMemo(
    () =>
      createFormDraftController({
        binding: identity,
        entityId,
        entityType,
        schema,
        state: () => ({
          ...current.current,
          available:
            adopted &&
            mounted.current &&
            current.current.identity === identity &&
            current.current.scope === scope &&
            current.current.schema === schema &&
            (current.current.enabled ?? true),
          writable: adopted && current.current.writable && !current.current.blocked,
        }),
        changed: (before, after, fields) => {
          appliedValues.current = JSON.stringify(after.values);
          setChanges((previous) => {
            const marks = new Map(
              previous.scope === scope && previous.baseline === current.current.baseline
                ? previous.fields.map((item) => [item.field, item])
                : [],
            );
            for (const field of fields) {
              const existing = marks.get(field);
              const preceding = before.values[field];
              const from =
                existing && JSON.stringify(existing.to) === JSON.stringify(preceding)
                  ? existing.from
                  : preceding;
              if (JSON.stringify(from) === JSON.stringify(after.values[field])) marks.delete(field);
              else marks.set(field, { field, from, to: after.values[field] });
            }
            return { scope, baseline: current.current.baseline, fields: [...marks.values()] };
          });
          setError(null);
        },
      }),
    [scope, schema, adopted, entityId, entityType, identity],
  );
  useLayoutEffect(() => {
    adapter.settled();
    appliedValues.current = null;
  });
  const register = mode?.registerPage;
  useEffect(() => {
    if (register && enabled)
      return register(
        {
          route,
          label: options.label,
          entity_id: options.entityId,
          entity_type: options.entityType,
          form_name: options.formId,
          editing: true,
        },
        adapter,
        options.priority ?? 20,
      );
  }, [
    register,
    enabled,
    route,
    options.label,
    options.entityId,
    options.entityType,
    options.formId,
    options.priority,
    adapter,
  ]);
  const marks =
    changes.scope === scope && changes.baseline === options.baseline ? changes.fields : [];
  function undo() {
    try {
      const value = adapter.read(adapter.describe().binding);
      if (
        marks.some((mark) => JSON.stringify(value.values[mark.field]) !== JSON.stringify(mark.to))
      )
        throw new Error("You edited a Chat-changed field. Undo would overwrite your edit.");
      if (marks.length)
        adapter.patch(
          value.binding,
          value.revision,
          Object.fromEntries(marks.map((mark) => [mark.field, mark.from])),
        );
      setChanges({ scope, baseline: options.baseline, fields: [] });
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not undo the draft edit.");
    }
  }
  const notice =
    mode && enabled && (marks.length || blocked || error) ? (
      <div className="my-3 border border-accent/50 bg-accent/5 p-3 text-sm" role="status">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">
            {blocked ? "The saved version changed" : "Draft edited by Chat"}
          </span>
          <span className="text-muted-foreground">
            {blocked
              ? "Your draft is kept. Discard it to review the saved version before saving."
              : "Review these changes, then save to apply them."}
          </span>
          {!!marks.length && (
            <Button type="button" size="sm" variant="ghost" onClick={undo}>
              Undo
            </Button>
          )}
          {!!marks.length && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => setChanges({ scope, baseline: options.baseline, fields: [] })}
            >
              Dismiss
            </Button>
          )}
        </div>
        {!!marks.length && (
          <p className="mt-1 text-muted-foreground">
            Changed: {marks.map((mark) => mark.field.replaceAll("_", " ")).join(", ")}
          </p>
        )}
        {error && (
          <p role="alert" className="mt-2 text-destructive">
            {error}
          </p>
        )}
      </div>
    ) : null;
  return {
    adopted,
    adapter,
    notice,
    blocked,
    clear: () => setChanges({ scope, baseline: options.baseline, fields: [] }),
  };
}
