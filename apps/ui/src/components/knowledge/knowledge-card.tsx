"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { FolderOpen } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { EntityCard, EntityCardDescription, EntityCardFooter } from "@/components/ui/entity-card";
import { EntityStatus } from "@/components/ui/entity-status";
import { collectionState } from "@/components/knowledge/knowledge-status";
import { indexingLabel, sourceSummary } from "@/components/knowledge/use-knowledge";
import { formatRelativeTime } from "@/lib/formatting";
import type { KnowledgeCollection } from "@/lib/api/knowledge";

export function KnowledgeCard({ collection }: { collection: KnowledgeCollection }) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const state = collectionState(collection);
  const synced = (collection.sources ?? [])
    .map((source) => source.last_synced_at)
    .filter((value): value is string => !!value)
    .sort()
    .at(-1);
  const meta = [
    collection.file_count !== undefined &&
      ui("{value0} files", { value0: collection.file_count.toLocaleString(displayLocale) }),
    collection.indexing !== "off" &&
      collection.indexed_count !== undefined &&
      ui("{value0} indexed", { value0: collection.indexed_count.toLocaleString(displayLocale) }),
    synced && ui("Synced {value0}", { value0: formatRelativeTime(synced, displayLocale) }),
  ].filter(Boolean);
  return (
    <EntityCard
      href={`/knowledge/${collection.id}`}
      icon={<FolderOpen className="icon-sharp size-5 text-muted-foreground" />}
      title={collection.name}
      subtitle={ui(sourceSummary(collection))}
      headerActions={
        <div className="flex flex-wrap items-center justify-end gap-2">
          {state && <EntityStatus status={state.label} tone={state.tone} label={state.label} />}
          <Badge variant="secondary">{ui(indexingLabel(collection.indexing))}</Badge>
        </div>
      }
      footer={meta.length ? <EntityCardFooter meta={meta.join(" · ")} /> : undefined}
    >
      <EntityCardDescription>
        {collection.description ||
          ui("Add files or connect a source to give agents useful context.")}
      </EntityCardDescription>
    </EntityCard>
  );
}
