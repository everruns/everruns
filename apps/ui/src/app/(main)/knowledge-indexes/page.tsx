"use client";

import Link from "next/link";
import { useState } from "react";
import {
  AlertCircle,
  Archive,
  FolderOpen,
  GitBranch,
  Library,
  Pencil,
  Plus,
  RefreshCw,
  Search,
} from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { ArchiveFilter } from "@/components/archive-filter";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ArchiveKnowledgeIndexDialog } from "@/components/knowledge-indexes/archive-knowledge-index-dialog";
import { KnowledgeIndexFormDialog } from "@/components/knowledge-indexes/knowledge-index-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Input } from "@/components/ui/input";
import {
  useArchiveKnowledgeIndex,
  useCreateKnowledgeIndex,
  useKnowledgeIndexes,
  usePageTitle,
  useSyncKnowledgeIndex,
  useUpdateKnowledgeIndex,
} from "@/hooks";
import type {
  CreateKnowledgeIndexRequest,
  KnowledgeIndex,
  UpdateKnowledgeIndexRequest,
} from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatRelativeTime } from "@/lib/formatting";
import { syncStatusBadgeVariant } from "@/lib/knowledge-index-sync";

export default function KnowledgeIndexesPage() {
  usePageTitle("Knowledge Indexes");
  const [showArchived, setShowArchived] = useState(false);
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [editingIndex, setEditingIndex] = useState<KnowledgeIndex | null>(null);
  const [archivingIndex, setArchivingIndex] = useState<KnowledgeIndex | null>(null);
  const {
    data: indexes,
    isLoading,
    error,
  } = useKnowledgeIndexes({ includeArchived: showArchived, search });
  const createIndex = useCreateKnowledgeIndex();
  const updateIndex = useUpdateKnowledgeIndex();
  const syncIndex = useSyncKnowledgeIndex();
  const archiveIndex = useArchiveKnowledgeIndex();

  return (
    <div className="container mx-auto p-6">
      <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
        <h1 className="flex items-center gap-3 text-2xl font-bold">Knowledge Indexes</h1>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <div className="relative sm:w-64">
            <Search className="pointer-events-none absolute left-2.5 top-2 h-4 w-4 text-muted-foreground" />
            <Input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search knowledge indexes"
              className="pl-8"
              aria-label="Search knowledge indexes"
            />
          </div>
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Button variant="accent" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            New Index
          </Button>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={indexes}
        errorMessagePrefix="Failed to load knowledge indexes"
        skeletonCount={6}
        emptyState={<EmptyState hasSearch={!!search.trim()} onCreate={() => setCreateOpen(true)} />}
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {items.map((index) => (
              <KnowledgeIndexCard
                key={index.id}
                index={index}
                onEdit={setEditingIndex}
                onArchive={setArchivingIndex}
                onSync={(candidate) => syncIndex.mutate(candidate.id)}
                isSyncing={syncIndex.isPending}
              />
            ))}
          </div>
        )}
      </QueryStateWrapper>

      <KnowledgeIndexFormDialog
        mode="create"
        open={createOpen}
        isPending={createIndex.isPending}
        onOpenChange={setCreateOpen}
        onSubmit={async (request) => {
          await createIndex.mutateAsync(request as CreateKnowledgeIndexRequest);
        }}
      />
      <KnowledgeIndexFormDialog
        mode="edit"
        open={!!editingIndex}
        index={editingIndex}
        isPending={updateIndex.isPending}
        onOpenChange={(open) => !open && setEditingIndex(null)}
        onSubmit={async (request) => {
          await updateIndex.mutateAsync({
            indexId: editingIndex!.id,
            data: request as UpdateKnowledgeIndexRequest,
          });
        }}
      />
      <ArchiveKnowledgeIndexDialog
        open={!!archivingIndex}
        index={archivingIndex}
        isPending={archiveIndex.isPending}
        onOpenChange={(open) => !open && setArchivingIndex(null)}
        onArchive={() => archiveIndex.mutateAsync(archivingIndex!.id)}
      />
    </div>
  );
}

function KnowledgeIndexCard({
  index,
  onEdit,
  onArchive,
  onSync,
  isSyncing,
}: {
  index: KnowledgeIndex;
  onEdit: (index: KnowledgeIndex) => void;
  onArchive: (index: KnowledgeIndex) => void;
  onSync: (index: KnowledgeIndex) => void;
  isSyncing: boolean;
}) {
  const isReadOnly = isReadOnlyStatus(index.status);
  const canSync = index.status === "active";

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center bg-primary/10">
            <Library className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <CardTitle className={`truncate text-lg ${getEntityNameClassName(index.status)}`}>
              {index.name}
            </CardTitle>
            <div className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
              <span className="truncate font-mono">{index.id}</span>
              <CopyButton value={index.id} />
            </div>
          </div>
        </div>
        <Badge variant={getEntityStatusBadgeVariant(index.status)}>{index.status}</Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="min-h-10 text-sm text-muted-foreground">
          {index.description || "No description"}
        </p>
        <div className="grid grid-cols-2 gap-3 text-xs text-muted-foreground">
          <div>
            <div className="font-medium text-foreground">Source</div>
            <div className="flex items-center gap-1">
              {index.source_type === "github" ? (
                <Github className="h-3.5 w-3.5" />
              ) : (
                <GitBranch className="h-3.5 w-3.5" />
              )}
              <span>{index.source_type}</span>
            </div>
          </div>
          <div>
            <div className="font-medium text-foreground">Sync</div>
            <Badge variant={syncStatusBadgeVariant(index.sync_status)}>{index.sync_status}</Badge>
          </div>
          <div>
            <div className="font-medium text-foreground">Last synced</div>
            <div>{index.last_synced_at ? formatRelativeTime(index.last_synced_at) : "Never"}</div>
          </div>
          <div>
            <div className="font-medium text-foreground">Updated</div>
            <div>{formatRelativeTime(index.updated_at)}</div>
          </div>
        </div>
        {index.last_sync_error && (
          <div className="flex gap-2 text-xs text-destructive">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="line-clamp-2">{index.last_sync_error}</span>
          </div>
        )}
        <div className="flex items-center justify-end gap-2">
          {canSync && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onSync(index)}
              disabled={
                isSyncing || index.sync_status === "pending" || index.sync_status === "syncing"
              }
            >
              <RefreshCw className="h-4 w-4" />
              Sync
            </Button>
          )}
          <Link href={`/knowledge-indexes/${index.id}`}>
            <Button variant="outline" size="sm">
              <FolderOpen className="h-4 w-4" />
              Open
            </Button>
          </Link>
          <Button variant="outline" size="sm" onClick={() => onEdit(index)} disabled={isReadOnly}>
            <Pencil className="h-4 w-4" />
            Edit
          </Button>
          {!isReadOnly && (
            <Button variant="outline" size="sm" onClick={() => onArchive(index)}>
              <Archive className="h-4 w-4" />
              Archive
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function EmptyState({ hasSearch, onCreate }: { hasSearch: boolean; onCreate: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <Library className="mb-4 h-12 w-12 text-muted-foreground" />
      <h3 className="mb-2 text-lg font-semibold">
        {hasSearch ? "No knowledge indexes found" : "No knowledge indexes"}
      </h3>
      {!hasSearch && (
        <Button variant="accent" onClick={onCreate}>
          <Plus className="h-4 w-4" />
          New Index
        </Button>
      )}
    </div>
  );
}
