"use client";

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";
import { EntityStatus } from "@/components/ui/entity-status";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
import { pluralize } from "@/lib/formatting";
import type { Observer } from "@/lib/api/types";

export function ObserverCard({ observer }: { observer: Observer }) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  return (
    <EntityCard
      className="h-full"
      title={observer.name}
      href={`/observers/${observer.id}`}
      headerActions={<EntityStatus status={observer.status} />}
    >
      <div className="space-y-2">
        {observer.description && (
          <EntityCardDescription>{observer.description}</EntityCardDescription>
        )}
        <div className="flex items-center gap-4 text-xs text-muted-foreground">
          <span>
            {Math.round(observer.sampling_rate * 100)}
            {ui("% sampled")}
          </span>
          <span>
            {observer.scorers.length}{" "}
            {pluralize(observer.scorers.length, "scorer", undefined, displayLocale)}
          </span>
        </div>
      </div>
    </EntityCard>
  );
}
