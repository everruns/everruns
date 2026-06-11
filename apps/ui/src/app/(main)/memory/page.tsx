"use client";

import Link from "next/link";
import { useState } from "react";
import {
  AlertCircle,
  Archive,
  FolderOpen,
  GitBranch,
  HardDrive,
  Pencil,
  Plus,
  RefreshCw,
  Search,
} from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { ArchiveFilter } from "@/components/archive-filter";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ArchiveMemoryDialog } from "@/components/memory/archive-memory-dialog";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Input } from "@/components/ui/input";
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
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatRelativeTime } from "@/lib/formatting";

export default function MemoryPage() {
  usePageTitle("Memory");
  const [showArchived, setShowArchived] = useState(false);
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [editingMemory, setEditingMemory] = useState<Memory | null>(null);
  const [archivingMemory, setArchivingMemory] = useState<Memory | null>(null);
  const { data: memory, isLoading, error } = useMemories({ includeArchived: showArchived, search });
  const createMemory = useCreateMemory();
  const updateMemory = useUpdateMemory();
  const syncMemory = useSyncMemory();
  const archiveMemory = useArchiveMemory();

  return (
    <div className="container mx-auto p-6">
      <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
        <h1 className="flex items-center gap-3 text-2xl font-bold">Memory</h1>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <div className="relative sm:w-64">
            <Search className="pointer-events-none absolute left-2.5 top-2 h-4 w-4 text-muted-foreground" />
            <Input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search memory"
              className="pl-8"
              aria-label="Search memory"
            />
          </div>
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Button variant="accent" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            New Memory
          </Button>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={memory}
        errorMessagePrefix="Failed to load memory"
        skeletonCount={6}
        emptyState={<EmptyState hasSearch={!!search.trim()} onCreate={() => setCreateOpen(true)} />}
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
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
    </div>
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
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center bg-primary/10">
            <HardDrive className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <CardTitle className={`truncate text-lg ${getEntityNameClassName(memory.status)}`}>
              {memory.name}
            </CardTitle>
            <div className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
              <span className="truncate font-mono">{memory.id}</span>
              <CopyButton value={memory.id} />
            </div>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <Badge variant={getEntityStatusBadgeVariant(memory.status)}>{memory.status}</Badge>
          {memory.is_readonly && <Badge variant="secondary">Read-only</Badge>}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="min-h-10 text-sm text-muted-foreground">
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
          <div className="flex gap-2 text-xs text-destructive">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="line-clamp-2">{memory.last_sync_error}</span>
          </div>
        )}
        <div className="flex items-center justify-end gap-2">
          {canSync && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onSync(memory)}
              disabled={
                isSyncing || memory.sync_status === "pending" || memory.sync_status === "syncing"
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
          <Button variant="outline" size="sm" onClick={() => onEdit(memory)} disabled={isReadOnly}>
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
      </CardContent>
    </Card>
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

function EmptyState({ hasSearch, onCreate }: { hasSearch: boolean; onCreate: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <HardDrive className="mb-4 h-12 w-12 text-muted-foreground" />
      <h3 className="mb-2 text-lg font-semibold">{hasSearch ? "No memory found" : "No memory"}</h3>
      {!hasSearch && (
        <Button variant="accent" onClick={onCreate}>
          <Plus className="h-4 w-4" />
          New Memory
        </Button>
      )}
    </div>
  );
}
