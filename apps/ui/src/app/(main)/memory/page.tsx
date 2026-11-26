"use client";

// Memory (knowledge/runtime-resources/memory.md): every store agents use,
// grouped by who shares it. A store is a folder of Markdown files; typed
// entries (decisions, rules, facts, preferences, blocked items, notes) are
// files in kind folders. Named memories are created here and attached to
// agents in the Memory capability; agent, team and private memories appear
// on first use.

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";
import { MemoryIcon } from "@/components/icons/facet-icons";
import { useMemo, useState } from "react";
import { Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
import { SearchInput } from "@/components/ui/search-input";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageMain,
  IconTile,
} from "@/components/layout";
import { useCreateMemory, useMemoryStores, usePageTitle } from "@/hooks";
import type { CreateMemoryRequest } from "@/lib/api/types";
import type { MemoryStoreView } from "@/lib/api/memory";
import { formatRelativeTime } from "@/lib/formatting";
import {
  memoryStoreGroup,
  memoryStoreScopeLabel,
  memoryStoreTitle,
  type MemoryStoreGroup,
} from "@/components/memory/memory-store-labels";

type GroupTab = "all" | MemoryStoreGroup;

export default function MemoryPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Memory"));
  const { currentOrg } = useOrg();
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  const [tab, setTab] = useState<GroupTab>("all");
  const [search, setSearch] = useState("");
  useChatPageFilters({ filter: tab, search });
  const [createOpen, setCreateOpen] = useState(false);
  const { data: stores, isLoading, error } = useMemoryStores();
  const createMemory = useCreateMemory();

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return (stores ?? []).filter((store) => {
      if (tab !== "all" && memoryStoreGroup(store) !== tab) return false;
      if (!needle) return true;
      return [memoryStoreTitle(store), store.store, store.description ?? "", store.about]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [stores, tab, search]);

  const tabs = [
    { value: "all" as const, label: ui("All") },
    { value: "shared" as const, label: ui("Shared") },
    { value: "agents" as const, label: ui("Agents") },
    { value: "yours" as const, label: ui("Yours") },
  ];

  const newMemory = canManage && (
    <Button variant="accent" onClick={() => setCreateOpen(true)}>
      <Plus className="size-4" />
      {ui("New named memory")}
    </Button>
  );

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Memory") }]} />

      <PageMasthead
        icon={<MemoryIcon />}
        title={ui("Memory")}
        description={ui(
          "What agents remember between sessions: decisions, rules, facts, preferences and blocked items, kept as Markdown files with every change.",
        )}
        actions={newMemory}
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          containerClassName="w-64"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={ui("Search memory")}
          aria-label={ui("Search memory")}
        />
        <div className="flex-1" />
        <SectionTabs
          value={tab}
          onValueChange={(value) => setTab(value as GroupTab)}
          items={tabs}
        />
      </PageControlStrip>

      <PageMain>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={filtered}
          errorMessagePrefix="Failed to load memory"
          skeletonCount={6}
          emptyState={
            <EmptyState
              icon={<MemoryIcon />}
              title={search.trim() ? ui("No memory found") : ui("No memory yet")}
              description={
                search.trim()
                  ? undefined
                  : ui(
                      "Agents with the Memory capability create their memory the first time they run. Named memories are shared by the agents you attach them to.",
                    )
              }
              action={!search.trim() && newMemory}
            />
          }
        >
          {(items) => (
            <div className="grid gap-4 xl:grid-cols-2">
              {items.map((store) => (
                <MemoryStoreCard key={store.id} store={store} />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </PageMain>

      <MemoryFormDialog
        writable={canManage}
        mode="create"
        open={createOpen}
        isPending={createMemory.isPending}
        onOpenChange={setCreateOpen}
        onSubmit={async (request) => {
          await createMemory.mutateAsync(request as CreateMemoryRequest);
        }}
      />
    </PageContainer>
  );
}

function MemoryStoreCard({ store }: { store: MemoryStoreView }) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  return (
    <EntityCard
      icon={<IconTile size="md" icon={<MemoryIcon />} />}
      title={memoryStoreTitle(store)}
      href={`/memory/${store.id}`}
      subtitle={<span className="font-mono text-xs">{store.mount_path}</span>}
      headerActions={
        <div className="flex flex-col items-end gap-1">
          <Badge variant="secondary">{ui(memoryStoreScopeLabel(store))}</Badge>
          {store.blocked_count > 0 && (
            <Badge variant="accent">
              {store.blocked_count} {ui("blocked")}
            </Badge>
          )}
          {store.proposal_count > 0 && (
            <Badge variant="outline">
              {store.proposal_count} {ui("to review")}
            </Badge>
          )}
        </div>
      }
    >
      <EntityCardDescription>{store.description || ui(store.about)}</EntityCardDescription>
      <div className="grid grid-cols-3 gap-3 text-xs text-muted-foreground">
        <div>
          <div className="font-medium text-foreground">{ui("Entries")}</div>
          <div>{store.entry_count}</div>
        </div>
        <div>
          <div className="font-medium text-foreground">{ui("Files")}</div>
          <div>{store.file_count}</div>
        </div>
        <div>
          <div className="font-medium text-foreground">{ui("Changed")}</div>
          <div>{formatRelativeTime(store.last_changed_at, displayLocale)}</div>
        </div>
      </div>
    </EntityCard>
  );
}
