"use client";
import { useState } from "react";
import Link from "next/link";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, FolderOpen, Plus } from "lucide-react";
import { PageContainer, PageBreadcrumb, PageMasthead, PageMain } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { CollectionForm } from "@/components/knowledge/collection-form";
import { useOrg } from "@/providers/org-provider";
import { usePageTitle } from "@/hooks/use-page-title";
import { listKnowledge, createKnowledge } from "@/lib/api/knowledge";
export default function KnowledgePage() {
  usePageTitle("Knowledge");
  const { currentOrg } = useOrg();
  const cache = useQueryClient();
  const [creating, setCreating] = useState(false);
  const collections = useInfiniteQuery({
    queryKey: ["knowledge", currentOrg?.public_id],
    queryFn: ({ pageParam }) => listKnowledge(pageParam),
    initialPageParam: "",
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: !!currentOrg,
  });
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Knowledge" }]} />
      <PageMasthead
        icon={<BookOpen />}
        title="Knowledge"
        description="Files, context and connected sources. Index only what you need."
        actions={
          canManage ? (
            <Button variant="accent" onClick={() => setCreating(!creating)}>
              <Plus className="size-4" />
              New collection
            </Button>
          ) : undefined
        }
      />
      <PageMain>
        {creating && (
          <section className="mb-6 max-w-2xl rounded-lg border bg-card p-5">
            <h2 className="mb-4 text-lg font-medium">New collection</h2>
            <CollectionForm
              onCancel={() => setCreating(false)}
              onSave={async (input) => {
                await createKnowledge(input);
                setCreating(false);
                await cache.invalidateQueries({ queryKey: ["knowledge"] });
              }}
            />
          </section>
        )}
        {collections.isLoading && <p className="text-muted-foreground">Loading collections…</p>}
        {collections.error && (
          <p role="alert" className="text-destructive">
            Unable to load Knowledge. Check your organization permissions.
          </p>
        )}
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {collections.data?.pages
            .flatMap((p) => p.data)
            .map((c) => (
              <Link
                key={c.id}
                href={`/knowledge/${c.id}`}
                className="group rounded-lg border bg-card p-5 transition-colors hover:border-primary/50 focus-visible:outline-2 focus-visible:outline-ring"
              >
                <div className="flex items-center gap-3">
                  <FolderOpen className="size-5 text-muted-foreground" />
                  <h2 className="min-w-0 flex-1 truncate font-medium">{c.name}</h2>
                  <Badge variant="secondary">
                    {c.indexing === "off"
                      ? "Files"
                      : c.indexing === "lexical"
                        ? "Keyword"
                        : "Hybrid"}
                  </Badge>
                </div>
                <p className="mt-3 line-clamp-3 text-sm text-muted-foreground">
                  {c.description || "Add files or connect a source to give agents useful context."}
                </p>
                {c.name === "everruns-docs" && (
                  <p className="mt-4 text-xs text-muted-foreground">
                    Built-in · read without provider credentials
                  </p>
                )}
              </Link>
            ))}
        </div>
        {collections.hasNextPage && (
          <Button
            className="mt-4"
            variant="outline"
            disabled={collections.isFetchingNextPage}
            onClick={() => collections.fetchNextPage()}
          >
            Load more collections
          </Button>
        )}
      </PageMain>
    </PageContainer>
  );
}
