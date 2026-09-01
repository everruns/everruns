/**
 * Decisions:
 * - Mark a model switch inline in the transcript, the way compaction is marked: a model change
 *   silently alters every answer after it, so readers need it where it happened, not in settings.
 * - Names come from the event, not from the live model list: a renamed, disabled, or deleted model
 *   must still read correctly in an old session.
 */
"use client";

import { Cpu } from "lucide-react";
import type { SessionModelChangedData } from "@/lib/api/types";
import { useLocale } from "@/providers/locale-provider";

export function ModelChangeDivider({ data }: { data: SessionModelChangedData }) {
  const { t } = useLocale();
  const previous = data.previous_model_name ?? data.previous_model_id ?? t("default");

  return (
    <div className="flex w-full items-center gap-4 py-2 text-xs font-medium text-muted-foreground sm:text-sm">
      <div className="h-px flex-1 bg-border" />
      <span className="flex items-center gap-1.5 whitespace-nowrap">
        <Cpu className="h-3.5 w-3.5 shrink-0" />
        {t("model_changed", { from: previous, to: data.model_name })}
      </span>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}
