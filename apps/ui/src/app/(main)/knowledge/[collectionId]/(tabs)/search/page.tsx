"use client";

// Search tab: the passage search agents get from the Knowledge tool, against
// this collection only. A result opens its file in the Files tab.

import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";
import { use, useState } from "react";
import Link from "next/link";
import { Search } from "lucide-react";
import { KnowledgeIcon } from "@/components/icons/facet-icons";
import { EmptyState, PageMain } from "@/components/layout";
import { Button, LinkButton } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  useCanManageKnowledge,
  useKnowledgeCollection,
} from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";
import { searchKnowledge, type KnowledgeResults } from "@/lib/api/knowledge";

export default function KnowledgeSearchPage({
  params,
}: {
  params: Promise<{ collectionId: string }>;
}) {
  const ui = useUiTranslations();
  const { collectionId } = use(params);
  const collection = useKnowledgeCollection(collectionId);
  const canManage = useCanManageKnowledge();
  usePageTitle(ui("Search"), collection.data?.name ?? ui("Knowledge"));
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<KnowledgeResults | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useChatPageFilters({ search: query });
  const c = collection.data;

  if (c?.indexing === "off") {
    return (
      <PageMain>
        <EmptyState
          icon={<KnowledgeIcon />}
          title={ui("Indexing is off")}
          description={ui(
            "Agents can still list, read and grep these files. Turn on keyword or hybrid indexing to search by passage.",
          )}
          action={
            canManage ? (
              <LinkButton variant="outline" href={`/knowledge/${collectionId}/settings`}>
                {ui("Open settings")}
              </LinkButton>
            ) : undefined
          }
        />
      </PageMain>
    );
  }

  return (
    <PageMain className="max-w-4xl space-y-4">
      <form
        className="flex gap-2"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError("");
          try {
            setResults(await searchKnowledge(collectionId, query));
          } catch (err) {
            setError(err instanceof Error ? err.message : "Search failed.");
          } finally {
            setBusy(false);
          }
        }}
      >
        <Input
          aria-label={ui("Search collection")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={ui("Find relevant passages…")}
        />
        <Button type="submit" variant="accent" disabled={busy || !query.trim() || !c}>
          <Search className="size-4" />
          {ui("Search")}
        </Button>
      </form>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {results && (
        <div className="space-y-3">
          {results.warnings.map((warning) => (
            <p key={warning} className="text-xs text-muted-foreground">
              {warning}
            </p>
          ))}
          {results.results.map((hit) => (
            <Card key={hit.id} className="gap-1 p-4">
              <Link
                href={`/knowledge/${collectionId}?${new URLSearchParams({ file: hit.path })}`}
                className="break-all font-mono text-xs text-muted-foreground hover:text-foreground hover:underline"
              >
                {hit.path}
              </Link>
              <h3 className="text-sm font-medium">{hit.title}</h3>
              <p className="whitespace-pre-wrap text-sm text-foreground/75">{hit.snippet}</p>
              {/^https:\/\//.test(hit.source_uri) && (
                <a
                  className="text-xs text-muted-foreground underline"
                  href={hit.source_uri}
                  target="_blank"
                  rel="noreferrer"
                >
                  {ui("Original source")}
                </a>
              )}
            </Card>
          ))}
          {!results.results.length && (
            <p className="text-sm text-muted-foreground">{ui("No matching indexed passages.")}</p>
          )}
          {results.truncated && (
            <p className="text-xs text-muted-foreground">
              {ui("Results are bounded. Narrow the query for more complete coverage.")}
            </p>
          )}
        </div>
      )}
    </PageMain>
  );
}
