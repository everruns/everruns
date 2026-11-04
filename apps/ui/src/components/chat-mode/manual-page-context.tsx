"use client";

import { usePageReadContext } from "./use-page-read-context";

// Mount inside the popup, so closing it restores the preceding page binding.
// This masks underlying forms without reading protected inputs or page HTML.
export function ManualPageContext({ label }: { label: string }) {
  usePageReadContext({
    fields: ["interaction"],
    values: {
      interaction:
        "Use this surface's manual controls; its protected values and actions are unavailable to Chat.",
    },
    entityType: "manual_dialog",
    label,
    formId: "manual_dialog",
    priority: 100,
  });
  return null;
}
