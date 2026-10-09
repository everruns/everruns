"use client";

import { MemoryIcon } from "@/components/icons/facet-icons";
import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import {
  AlertCircle,
  Archive,
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
import { Button, LinkButton } from "@/components/ui/button";
import { EntityCard, EntityCardFooter, EntityCardDescription } from "@/components/ui/entity-card";
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
import {
  useArchiveMemory,
  useCreateMemory,
  usePageTitle,
  useSyncMemory,
  useUpdateMemory,
  useMemories,
} from "@/hooks";
import type { CreateMemoryRequest, UpdateMemoryRequest, Memory } from "@/lib/api/types";
import { getEntityNameClassName, isArchivedStatus, isReadOnlyStatus } from "@/lib/entity-lifecycle";
import { formatRelativeTime } from "@/lib/formatting";

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

  const statusItems = [
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Memory" }]} />

      <PageMasthead
        icon={<MemoryIcon />}
        title="Memory"
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

      <PageMain>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={filteredMemory}
          errorMessagePrefix="Failed to load memory"
          skeletonCount={6}
          emptyState={
            <EmptyState
              icon={<MemoryIcon />}
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
      icon={<IconTile size="md" icon={<MemoryIcon />} />}
      title={memory.name}
      href={`/memory/${memory.id}`}
      titleClassName={getEntityNameClassName(memory.status)}
      headerActions={
        <div className="flex flex-col items-end gap-1">
          <EntityStatus status={memory.status} />
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
              <LinkButton variant="outline" size="sm" href={`/memory/${memory.id}`}>
                <FolderOpen className="h-4 w-4" />
                Open
              </LinkButton>
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
      <EntityCardDescription>{memory.description || "No description"}</EntityCardDescription>
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
