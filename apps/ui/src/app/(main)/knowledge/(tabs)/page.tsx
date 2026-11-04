"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { KnowledgeCard } from "@/components/knowledge/knowledge-card";
import { KnowledgeIcon } from "@/components/icons/facet-icons";
import { EmptyState, PageMain } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { useKnowledgeCollections } from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";
import { useKnowledgeCollectionRows } from "@/components/knowledge/use-knowledge-page-context";

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
              <KnowledgeCard key={collection.id} collection={collection} />
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
