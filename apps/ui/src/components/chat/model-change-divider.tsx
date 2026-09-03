/**
 * Decisions:
 * - Mark a model switch inline in the transcript, the way compaction is marked: a model change
 *   silently alters every answer after it, so readers need it where it happened, not in settings.
 * - Prefer the org's display name for a model still in the list; fall back to the name the event
 *   captured (the provider's own model id) so an old session stays readable after a model is
 *   renamed or removed.
 */
"use client";

import { Cpu } from "lucide-react";
import type { SessionModelChangedData } from "@/lib/api/types";
import { useModels } from "@/hooks";
import { useLocale } from "@/providers/locale-provider";

export function ModelChangeDivider({ data }: { data: SessionModelChangedData }) {
  const { t } = useLocale();
  const { data: models = [] } = useModels();
  const displayName = (modelId: string | undefined, captured: string | undefined) =>
    (modelId && models.find((model) => model.id === modelId)?.display_name) ?? captured;

  const from =
    displayName(data.previous_model_id, data.previous_model_name) ??
    data.previous_model_id ??
    t("default");
  const to = displayName(data.model_id, data.model_name) ?? data.model_id;

  return (
    <div className="flex w-full items-center gap-4 py-2 text-xs font-medium text-muted-foreground sm:text-sm">
      <div className="h-px flex-1 bg-border" />
      <span className="flex items-center gap-1.5 whitespace-nowrap">
        <Cpu className="h-3.5 w-3.5 shrink-0" />
        {t("model_changed", { from, to })}
      </span>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}
