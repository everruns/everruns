"use client";

import { useTextPageFormDraft } from "./use-text-page-form-draft";

// Read-only and protected surfaces expose only an explicit public projection.
// Their immediate actions retain existing authorization and manual controls.
export function usePageReadContext<K extends string>(options: {
  fields: readonly K[];
  values: Record<K, string>;
  entityType: string;
  entityId?: string;
  label: string;
  enabled?: boolean;
  formId?: string;
  priority?: number;
}) {
  useTextPageFormDraft({
    ...options,
    formId: options.formId ?? "current_settings",
    baseline: "view",
    dirty: false,
    writable: false,
    apply: () => {
      throw new Error("Use this page's existing settings controls.");
    },
  });
}
