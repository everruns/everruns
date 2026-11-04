"use client";

import { useRef, useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { usePageChatMode } from "./chat-mode-context";
import { usePageFormDraft } from "./use-page-form-draft";

// Autosaving pages stage both mouse and Chat edits after Adoption. Only the
// rendered Save button receives the existing API callback; the adapter never does.
export function usePageSettingsDraft<T extends Record<string, unknown>>(options: {
  values: T;
  schema: z.ZodType<T>;
  entityType: string;
  entityId?: string;
  label: string;
  formId?: string;
  enabled?: boolean;
  writable: boolean;
  save: (values: T) => Promise<unknown>;
  saveLabel?: string;
}) {
  const mode = usePageChatMode();
  const key = `${mode?.page.route ?? ""}:${options.entityType}:${options.entityId ?? "new"}:${options.formId ?? "main"}`;
  const [override, setOverride] = useState<{ key: string; values: T } | null>(null);
  const [pending, setPending] = useState(false);
  const saving = useRef(false);
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);
  const values = override?.key === key ? override.values : options.values;
  const dirty = JSON.stringify(values) !== JSON.stringify(options.values);
  const setValues = (next: T) => {
    if (!saving.current) {
      setOverride(
        JSON.stringify(next) === JSON.stringify(options.values) ? null : { key, values: next },
      );
    }
  };
  const chat = usePageFormDraft({
    entityType: options.entityType,
    entityId: options.entityId,
    label: options.label,
    formId: options.formId,
    enabled: options.enabled,
    schema: options.schema,
    values,
    dirty,
    baseline: JSON.stringify(options.values),
    writable: options.writable && !pending,
    apply: setValues,
  });
  const discard = () => {
    if (saving.current) return;
    setOverride(null);
    setFailure(null);
    chat.clear();
  };
  const save = async () => {
    if (!chat.adopted || !dirty || chat.blocked || !options.writable || saving.current) return;
    saving.current = true;
    setPending(true);
    setFailure(null);
    try {
      const snapshot = options.schema.parse(values);
      await options.save(snapshot);
      setOverride((current) => (current?.key === key ? null : current));
      chat.clear();
    } catch (error) {
      setFailure({
        key,
        message: error instanceof Error ? error.message : "Could not save changes.",
      });
    } finally {
      saving.current = false;
      setPending(false);
    }
  };
  const controls = chat.adopted ? (
    <div className="space-y-2">
      {chat.notice}
      {failure?.key === key && (
        <p role="alert" className="text-sm text-destructive">
          {failure.message}
        </p>
      )}
      {dirty && (
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            disabled={chat.blocked || !options.writable || pending}
            onClick={() => void save()}
          >
            {pending ? "Saving…" : (options.saveLabel ?? "Save changes")}
          </Button>
          <Button type="button" variant="outline" disabled={pending} onClick={discard}>
            Discard
          </Button>
        </div>
      )}
    </div>
  ) : null;
  return { ...chat, values, setValues, dirty, pending, controls, discard };
}
