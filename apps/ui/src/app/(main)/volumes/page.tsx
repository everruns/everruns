"use client";

import Link from "next/link";
import { useState } from "react";
import {
  Archive,
  FolderOpen,
  GitBranch,
  Github,
  HardDrive,
  Pencil,
  Plus,
  Search,
} from "lucide-react";
import { ArchiveFilter } from "@/components/archive-filter";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ArchiveVolumeDialog } from "@/components/volumes/archive-volume-dialog";
import { VolumeFormDialog } from "@/components/volumes/volume-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Input } from "@/components/ui/input";
import {
  useArchiveVolume,
  useCreateVolume,
  usePageTitle,
  useUpdateVolume,
  useVolumes,
} from "@/hooks";
import type { CreateVolumeRequest, UpdateVolumeRequest, Volume } from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatRelativeTime } from "@/lib/formatting";

export default function VolumesPage() {
  usePageTitle("Volumes");
  const [showArchived, setShowArchived] = useState(false);
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [editingVolume, setEditingVolume] = useState<Volume | null>(null);
  const [archivingVolume, setArchivingVolume] = useState<Volume | null>(null);
  const { data: volumes, isLoading, error } = useVolumes({ includeArchived: showArchived, search });
  const createVolume = useCreateVolume();
  const updateVolume = useUpdateVolume();
  const archiveVolume = useArchiveVolume();

  return (
    <div className="container mx-auto p-6">
      <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
        <h1 className="flex items-center gap-3 text-2xl font-bold">Volumes</h1>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <div className="relative sm:w-64">
            <Search className="pointer-events-none absolute left-2.5 top-2 h-4 w-4 text-muted-foreground" />
            <Input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search volumes"
              className="pl-8"
              aria-label="Search volumes"
            />
          </div>
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Button variant="accent" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            New Volume
          </Button>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={volumes}
        errorMessagePrefix="Failed to load volumes"
        skeletonCount={6}
        emptyState={<EmptyState hasSearch={!!search.trim()} onCreate={() => setCreateOpen(true)} />}
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {items.map((volume) => (
              <VolumeCard
                key={volume.id}
                volume={volume}
                onEdit={setEditingVolume}
                onArchive={setArchivingVolume}
              />
            ))}
          </div>
        )}
      </QueryStateWrapper>

      <VolumeFormDialog
        mode="create"
        open={createOpen}
        isPending={createVolume.isPending}
        onOpenChange={setCreateOpen}
        onSubmit={async (request) => {
          await createVolume.mutateAsync(request as CreateVolumeRequest);
        }}
      />
      <VolumeFormDialog
        mode="edit"
        open={!!editingVolume}
        volume={editingVolume}
        isPending={updateVolume.isPending}
        onOpenChange={(open) => !open && setEditingVolume(null)}
        onSubmit={async (request) => {
          await updateVolume.mutateAsync({
            volumeId: editingVolume!.id,
            data: request as UpdateVolumeRequest,
          });
        }}
      />
      <ArchiveVolumeDialog
        open={!!archivingVolume}
        volume={archivingVolume}
        isPending={archiveVolume.isPending}
        onOpenChange={(open) => !open && setArchivingVolume(null)}
        onArchive={() => archiveVolume.mutateAsync(archivingVolume!.id)}
      />
    </div>
  );
}

function VolumeCard({
  volume,
  onEdit,
  onArchive,
}: {
  volume: Volume;
  onEdit: (volume: Volume) => void;
  onArchive: (volume: Volume) => void;
}) {
  const isReadOnly = isReadOnlyStatus(volume.status);

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center bg-primary/10">
            <HardDrive className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <CardTitle className={`truncate text-lg ${getEntityNameClassName(volume.status)}`}>
              {volume.name}
            </CardTitle>
            <div className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
              <span className="truncate font-mono">{volume.id}</span>
              <CopyButton value={volume.id} />
            </div>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <Badge variant={getEntityStatusBadgeVariant(volume.status)}>{volume.status}</Badge>
          {volume.is_readonly && <Badge variant="secondary">Read-only</Badge>}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="min-h-10 text-sm text-muted-foreground">
          {volume.description || "No description"}
        </p>
        <div className="grid grid-cols-2 gap-3 text-xs text-muted-foreground">
          <div>
            <div className="font-medium text-foreground">Source</div>
            <div className="flex items-center gap-1">
              {volume.source_type === "github" ? (
                <Github className="h-3.5 w-3.5" />
              ) : volume.source_type === "git" ? (
                <GitBranch className="h-3.5 w-3.5" />
              ) : (
                <HardDrive className="h-3.5 w-3.5" />
              )}
              <span>{volume.source_type === "manual" ? "Manual" : volume.source_type}</span>
            </div>
          </div>
          <div>
            <div className="font-medium text-foreground">Sync</div>
            <div>{volume.sync_status}</div>
          </div>
          <div>
            <div className="font-medium text-foreground">Created</div>
            <div>{formatRelativeTime(volume.created_at)}</div>
          </div>
          <div>
            <div className="font-medium text-foreground">Updated</div>
            <div>{formatRelativeTime(volume.updated_at)}</div>
          </div>
        </div>
        <div className="flex items-center justify-end gap-2">
          <Link href={`/volumes/${volume.id}`}>
            <Button variant="outline" size="sm">
              <FolderOpen className="h-4 w-4" />
              Open
            </Button>
          </Link>
          <Button variant="outline" size="sm" onClick={() => onEdit(volume)} disabled={isReadOnly}>
            <Pencil className="h-4 w-4" />
            Edit
          </Button>
          {!isReadOnly && (
            <Button variant="outline" size="sm" onClick={() => onArchive(volume)}>
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
      <HardDrive className="mb-4 h-12 w-12 text-muted-foreground" />
      <h3 className="mb-2 text-lg font-semibold">
        {hasSearch ? "No volumes found" : "No volumes"}
      </h3>
      {!hasSearch && (
        <Button variant="accent" onClick={onCreate}>
          <Plus className="h-4 w-4" />
          New Volume
        </Button>
      )}
    </div>
  );
}
