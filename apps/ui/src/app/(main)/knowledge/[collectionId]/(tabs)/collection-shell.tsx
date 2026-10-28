"use client";

// A Knowledge collection: one page with tabs, like Sandboxes. Files (the
// default) browses the collection with the session file browser, Search runs
// the passage search agents use, Sources lists where files come from, and
// Settings holds the collection form. Each tab is its own path under
// `/knowledge/{id}`; the shell is the tabs' layout.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useState, type ReactNode } from "react";
import { usePathname } from "next/navigation";
import { FolderTree, RefreshCw, Search, Settings2, Unplug } from "lucide-react";
import { KnowledgeIcon } from "@/components/icons/facet-icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { PageBreadcrumb, PageContainer, PageMasthead, SectionTabs } from "@/components/layout";
import { ResourceNotFound } from "@/components/resource-not-found";
import { EntityStatus } from "@/components/ui/entity-status";
import { collectionState } from "@/components/knowledge/knowledge-status";
import {
  indexingLabel,
  useCanManageKnowledge,
  useKnowledgeCollection,
  useRefreshKnowledge,
} from "@/components/knowledge/use-knowledge";
import { syncKnowledge } from "@/lib/api/knowledge";
import { formatRelativeTime } from "@/lib/formatting";
import { tabFromPath, tabHref } from "@/lib/tab-routes";

const TABS = ["files", "search", "sources", "settings"] as const;

export function CollectionShell({
  collectionId,
  children,
}: {
  collectionId: string;
  children: ReactNode;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const base = `/knowledge/${collectionId}`;
  const tab = tabFromPath(usePathname(), base, TABS, "files");
  const canManage = useCanManageKnowledge();
  const collection = useKnowledgeCollection(collectionId);
  const refresh = useRefreshKnowledge();
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState("");
  const c = collection.data;

  if (collection.error && !c) {
    return (
      <PageContainer>
        <ResourceNotFound
          title={ui("Collection not found")}
          description={ui("It may have been archived, or it belongs to another organization.")}
          backHref="/knowledge"
          backLabel={ui("Back to Knowledge")}
          resourceId={collectionId}
        />
      </PageContainer>
    );
  }

  const state = c ? collectionState(c) : null;
  const synced = (c?.sources ?? [])
    .map((source) => source.last_synced_at)
    .filter((value): value is string => !!value)
    .sort()
    .at(-1);
  const tabs = [
    {
      value: "files",
      href: tabHref(base, "files", "files"),
      label: ui("Files"),
      icon: <FolderTree className="icon-sharp size-3.5" />,
      count: c?.file_count,
    },
    {
      value: "search",
      href: tabHref(base, "search", "files"),
      label: ui("Search"),
      icon: <Search className="icon-sharp size-3.5" />,
    },
    {
      value: "sources",
      href: tabHref(base, "sources", "files"),
      label: ui("Sources"),
      icon: <Unplug className="icon-sharp size-3.5" />,
      count: c?.sources?.length,
    },
    ...(canManage
      ? [
          {
            value: "settings",
            href: tabHref(base, "settings", "files"),
            label: ui("Settings"),
            icon: <Settings2 className="icon-sharp size-3.5" />,
          },
        ]
      : []),
  ];

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Knowledge"), href: "/knowledge" }, { label: c?.name ?? "…" }]}
      />
      <PageMasthead
        icon={<KnowledgeIcon />}
        title={c?.name ?? ui("Knowledge")}
        badges={c && <Badge variant="secondary">{ui(indexingLabel(c.indexing))}</Badge>}
        description={c ? c.description : ui("Loading collection…")}
        meta={
          c && (
            <>
              <span>
                {ui("{value0} files", {
                  value0: (c.file_count ?? 0).toLocaleString(displayLocale),
                })}
                {c.indexing !== "off" &&
                  ` · ${ui("{value0} indexed", { value0: (c.indexed_count ?? 0).toLocaleString(displayLocale) })}`}
              </span>
              {synced && (
                <span>
                  {ui("Synced {value0}", { value0: formatRelativeTime(synced, displayLocale) })}
                </span>
              )}
              {state && <EntityStatus status={state.label} tone={state.tone} label={state.label} />}
            </>
          )
        }
        actions={
          canManage && (
            <Button
              variant="outline"
              disabled={syncing || !c}
              onClick={async () => {
                setSyncing(true);
                setError("");
                try {
                  await syncKnowledge(collectionId);
                  await refresh();
                } catch (e) {
                  setError(e instanceof Error ? e.message : "Sources could not be refreshed.");
                } finally {
                  setSyncing(false);
                }
              }}
            >
              <RefreshCw className="size-4" />
              {ui("Refresh sources")}
            </Button>
          )
        }
      />
      <SectionTabs aria-label={ui("Collection sections")} value={tab} items={tabs} />
      {error && (
        <p role="alert" className="mt-4 text-sm text-destructive">
          {error}
        </p>
      )}
      {children}
    </PageContainer>
  );
}
