"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import {
  AlertCircle,
  Archive,
  Brain,
  FolderOpen,
  GitBranch,
  HardDrive,
  Pencil,
  Plus,
  RefreshCw,
} from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ArchiveMemoryDialog } from "@/components/memory/archive-memory-dialog";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { SearchInput } from "@/components/ui/search-input";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
  IconTile,
} from "@/components/layout";
import {
  useArchiveMemory,
  useCreateMemory,
  usePageTitle,
  useSyncMemory,
  useUpdateMemory,
  useMemories,
} from "@/hooks";
import type { CreateMemoryRequest, UpdateMemoryRequest, Memory } from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isArchivedStatus,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatRelativeTime, pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";

type StatusTab = "active" | "archived";

export default function MemoryPage() {
  usePageTitle("Memory");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [editingMemory, setEditingMemory] = useState<Memory | null>(null);
  const [archivingMemory, setArchivingMemory] = useState<Memory | null>(null);
  const showArchived = statusTab === "archived";
  const { data: memory, isLoading, error } = useMemories({ includeArchived: showArchived, search });
  const createMemory = useCreateMemory();
  const updateMemory = useUpdateMemory();
  const syncMemory = useSyncMemory();
  const archiveMemory = useArchiveMemory();

  const list = useMemo(() => memory ?? [], [memory]);

  // When showing archived, the API includes both active and archived; narrow to
  // the selected tab so the count and grid stay consistent with the filter.
  const filteredMemory = useMemo(() => {
    if (!showArchived) return list;
    return list.filter((m) => isArchivedStatus(m.status));
  }, [list, showArchived]);

  const counts = useMemo(() => {
    const archived = list.filter((m) => isArchivedStatus(m.status)).length;
    return { active: list.length - archived, archived };
  }, [list]);

  // Source-type usage counts for the rail facet.
  const sourceFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const m of filteredMemory) {
      tally.set(m.source_type, (tally.get(m.source_type) ?? 0) + 1);
    }
    return [...tally.entries()].sort((a, b) => b[1] - a[1]);
  }, [filteredMemory]);

  const statusItems = [
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Memory" }]} />

      <PageMasthead
        icon={<Brain />}
        title="Memory"
        badges={
          <Badge variant="outline" className="font-mono">
            {filteredMemory.length}
          </Badge>
        }
        description="Knowledge stores that agents can read — manual notes or synced from Git."
        actions={
          <Button variant="accent" onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" />
            New Memory
          </Button>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          containerClassName="w-64"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search memory"
          aria-label="Search memory"
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={filteredMemory}
            errorMessagePrefix="Failed to load memory"
            skeletonCount={6}
            emptyState={
              <EmptyState
                icon={<Brain />}
                title={search.trim() ? "No memory found" : "No memory"}
                action={
                  !search.trim() && (
                    <Button variant="accent" onClick={() => setCreateOpen(true)}>
                      <Plus className="size-4" />
                      New Memory
                    </Button>
                  )
                }
              />
            }
          >
            {(items) => (
              <div className="grid gap-4 xl:grid-cols-2">
                {items.map((memory) => (
                  <MemoryCard
                    key={memory.id}
                    memory={memory}
                    onEdit={setEditingMemory}
                    onArchive={setArchivingMemory}
                    onSync={(candidate) => syncMemory.mutate(candidate.id)}
                    isSyncing={syncMemory.isPending}
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </PageMain>

        <PageRail>
          <RailSection label="Status">
            <div className="flex flex-col gap-1.5 text-[13px]">
              {statusItems.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => setStatusTab(item.value)}
                  className={cn(
                    "flex items-center justify-between transition-colors hover:text-foreground",
                    statusTab === item.value ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <span>{item.label}</span>
                  <span className="text-muted-foreground">{counts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>

          {sourceFacets.length > 0 && (
            <RailSection label="Source">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {sourceFacets.map(([source, count]) => (
                  <div key={source} className="flex items-center justify-between">
                    <span className="inline-flex items-center gap-1.5">
                      {source === "github" ? (
                        <Github className="size-3.5" />
                      ) : source === "git" ? (
                        <GitBranch className="size-3.5" />
                      ) : (
                        <HardDrive className="size-3.5" />
                      )}
                      {source === "manual" ? "Manual" : source}
                    </span>
                    <span>{count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredMemory.length} {pluralize(filteredMemory.length, "memory", "memories")}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            View archived →
          </button>
        )}
      </PageFooter>

      <MemoryFormDialog
        mode="create"
        open={createOpen}
        isPending={createMemory.isPending}
        onOpenChange={setCreateOpen}
        onSubmit={async (request) => {
          await createMemory.mutateAsync(request as CreateMemoryRequest);
        }}
      />
      <MemoryFormDialog
        mode="edit"
        open={!!editingMemory}
        memory={editingMemory}
        isPending={updateMemory.isPending}
        onOpenChange={(open) => !open && setEditingMemory(null)}
        onSubmit={async (request) => {
          await updateMemory.mutateAsync({
            memoryId: editingMemory!.id,
            data: request as UpdateMemoryRequest,
          });
        }}
      />
      <ArchiveMemoryDialog
        open={!!archivingMemory}
        memory={archivingMemory}
        isPending={archiveMemory.isPending}
        onOpenChange={(open) => !open && setArchivingMemory(null)}
        onArchive={() => archiveMemory.mutateAsync(archivingMemory!.id)}
      />
    </PageContainer>
  );
}

function MemoryCard({
  memory,
  onEdit,
  onArchive,
  onSync,
  isSyncing,
}: {
  memory: Memory;
  onEdit: (memory: Memory) => void;
  onArchive: (memory: Memory) => void;
  onSync: (memory: Memory) => void;
  isSyncing: boolean;
}) {
  const isReadOnly = isReadOnlyStatus(memory.status);
  const canSync = memory.source_type !== "manual" && memory.status === "active";

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<Brain />} />}
      title={memory.name}
      href={`/memory/${memory.id}`}
      titleClassName={getEntityNameClassName(memory.status)}
      copyValue={memory.id}
      headerActions={
        <div className="flex flex-col items-end gap-1">
          <Badge variant={getEntityStatusBadgeVariant(memory.status)}>{memory.status}</Badge>
          {memory.is_readonly && <Badge variant="secondary">Read-only</Badge>}
        </div>
      }
      footer={
        <EntityCardFooter
          className="mt-4"
          actions={
            <div className="flex items-center gap-2">
              {canSync && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => onSync(memory)}
                  disabled={
                    isSyncing ||
                    memory.sync_status === "pending" ||
                    memory.sync_status === "syncing"
                  }
                >
                  <RefreshCw className="h-4 w-4" />
                  Sync
                </Button>
              )}
              <Link href={`/memory/${memory.id}`}>
                <Button variant="outline" size="sm">
                  <FolderOpen className="h-4 w-4" />
                  Open
                </Button>
              </Link>
              <Button
                variant="outline"
                size="sm"
                onClick={() => onEdit(memory)}
                disabled={isReadOnly}
              >
                <Pencil className="h-4 w-4" />
                Edit
              </Button>
              {!isReadOnly && (
                <Button variant="outline" size="sm" onClick={() => onArchive(memory)}>
                  <Archive className="h-4 w-4" />
                  Archive
                </Button>
              )}
            </div>
          }
        />
      }
    >
      <p className="mb-3 min-h-10 text-sm text-muted-foreground">
        {memory.description || "No description"}
      </p>
      <div className="grid grid-cols-2 gap-3 text-xs text-muted-foreground">
        <div>
          <div className="font-medium text-foreground">Source</div>
          <div className="flex items-center gap-1">
            {memory.source_type === "github" ? (
              <Github className="h-3.5 w-3.5" />
            ) : memory.source_type === "git" ? (
              <GitBranch className="h-3.5 w-3.5" />
            ) : (
              <HardDrive className="h-3.5 w-3.5" />
            )}
            <span>{memory.source_type === "manual" ? "Manual" : memory.source_type}</span>
          </div>
        </div>
        <div>
          <div className="font-medium text-foreground">Sync</div>
          <div>{formatSyncStatus(memory)}</div>
        </div>
        <div>
          <div className="font-medium text-foreground">Created</div>
          <div>{formatRelativeTime(memory.created_at)}</div>
        </div>
        <div>
          <div className="font-medium text-foreground">Updated</div>
          <div>{formatRelativeTime(memory.updated_at)}</div>
        </div>
      </div>
      {memory.last_sync_error && (
        <div className="mt-4 flex gap-2 text-xs text-destructive">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="line-clamp-2">{memory.last_sync_error}</span>
        </div>
      )}
    </EntityCard>
  );
}

function formatSyncStatus(memory: Memory) {
  if (memory.source_type === "manual") {
    return "Manual";
  }
  const interval = formatSyncInterval(memory);
  if (memory.last_synced_at) {
    return `${memory.sync_status} · ${formatRelativeTime(memory.last_synced_at)} · ${interval}`;
  }
  return `${memory.sync_status} · ${interval}`;
}

function formatSyncInterval(memory: Memory) {
  const interval =
    memory.source.provider === "github" || memory.source.provider === "git"
      ? memory.source.sync_interval_secs
      : null;
  if (!interval) {
    return "Manual only";
  }
  if (interval % 86400 === 0) {
    const days = interval / 86400;
    return days === 1 ? "Daily" : `Every ${days} days`;
  }
  if (interval % 3600 === 0) {
    const hours = interval / 3600;
    return hours === 1 ? "Hourly" : `Every ${hours} hours`;
  }
  return `Every ${Math.round(interval / 60)} minutes`;
}
