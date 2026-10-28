"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { FolderOpen } from "lucide-react";
import { KnowledgeIcon } from "@/components/icons/facet-icons";
import { EmptyState, PageMain } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardDescription, EntityCardFooter } from "@/components/ui/entity-card";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { EntityStatus } from "@/components/ui/entity-status";
import { collectionState } from "@/components/knowledge/knowledge-status";
import {
  indexingLabel,
  sourceSummary,
  useKnowledgeCollections,
} from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";
import { useKnowledgeCollectionRows } from "@/components/knowledge/use-knowledge-page-context";
import { formatRelativeTime } from "@/lib/formatting";
import type { KnowledgeCollection } from "@/lib/api/knowledge";

// Collections tab. The page header, tabs and "New collection" action come from
// the shell (./knowledge-shell.tsx).
export default function KnowledgeCollectionsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Knowledge"));
  const collections = useKnowledgeCollections();
  const items = collections.data?.pages.flatMap((page) => page.data);
  useKnowledgeCollectionRows(items ?? [], {
    loading: collections.isLoading,
    error: !!collections.error,
  });
  return (
    <PageMain>
      <QueryStateWrapper
        data={items}
        isLoading={collections.isLoading}
        error={collections.error}
        errorMessagePrefix="Unable to load Knowledge"
        emptyState={
          <EmptyState
            icon={<KnowledgeIcon />}
            title={ui("No collections yet")}
            description={ui(
              "A collection is a folder of files agents can read. Upload files, write notes or connect a Git repository.",
            )}
          />
        }
      >
        {(list) => (
          <div className="grid gap-4 xl:grid-cols-2">
            {list.map((collection) => (
              <CollectionCard key={collection.id} collection={collection} />
            ))}
          </div>
        )}
      </QueryStateWrapper>
      {collections.hasNextPage && (
        <Button
          className="mt-4"
          variant="outline"
          disabled={collections.isFetchingNextPage}
          onClick={() => collections.fetchNextPage()}
        >
          {ui("Load more collections")}
        </Button>
      )}
    </PageMain>
  );
}

function CollectionCard({ collection }: { collection: KnowledgeCollection }) {
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
