"use client";

import { useTextPageFormDraft } from "./use-text-page-form-draft";

// Immediate-save settings expose explicit public values without turning their
// onChange handlers into model-callable writes. Their normal controls retain
// their existing authorization and confirmation behavior.
export function usePageReadContext<K extends string>(options: {
  fields: readonly K[];
  values: Record<K, string>;
  entityType: string;
  entityId?: string;
  label: string;
  enabled?: boolean;
}) {
  useTextPageFormDraft({
    ...options,
    formId: "current_settings",
    baseline: "view",
    dirty: false,
    writable: false,
    apply: () => {
      throw new Error("Use this page's existing settings controls.");
    },
  });
}
