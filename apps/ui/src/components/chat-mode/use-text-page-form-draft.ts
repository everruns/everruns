"use client";

import { useMemo } from "react";
import { z } from "zod";
import { draftText, usePageFormDraft } from "./use-page-form-draft";

// Field names are an explicit developer-owned allowlist, independent from the
// form object. Passing additional state (including a credential) cannot expose it.
export function useTextPageFormDraft<K extends string>(options: {
  fields: readonly K[];
  values: Record<K, string>;
  apply: (values: Record<K, string>) => void;
  entityId?: string;
  entityType: string;
  label: string;
  baseline: string;
  dirty: boolean;
  writable: boolean;
  readonlyFields?: readonly K[];
  enabled?: boolean;
  formId?: string;
  priority?: number;
}) {
  const key = JSON.stringify(options.fields);
  const schema = useMemo(() => {
    const fields = JSON.parse(key) as string[];
    if (
      fields.length > 64 ||
      fields.some((field) => ["__proto__", "constructor", "prototype"].includes(field))
    )
      throw new Error("Invalid page form field allowlist.");
    const shape: Record<string, typeof draftText> = Object.fromEntries(
      fields.map((field) => [field, draftText]),
    );
    return z.strictObject(shape);
  }, [key]);
  const values = Object.fromEntries(options.fields.map((field) => [field, options.values[field]]));
  return usePageFormDraft({
    ...options,
    schema,
    values,
    // The schema above requires every key in this same allowlist.
    apply: (next) => options.apply(next as Record<K, string>),
  });
}
