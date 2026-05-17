"use client";

import Link from "next/link";
import { use, useState } from "react";
import {
  AlertCircle,
  Archive,
  ArrowLeft,
  GitBranch,
  Github,
  HardDrive,
  Pencil,
  RefreshCw,
} from "lucide-react";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ArchiveVolumeDialog } from "@/components/volumes/archive-volume-dialog";
import { VolumeFormDialog } from "@/components/volumes/volume-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import { useArchiveVolume, usePageTitle, useSyncVolume, useUpdateVolume, useVolume } from "@/hooks";
import type { UpdateVolumeRequest, Volume } from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatDate, formatRelativeTime } from "@/lib/formatting";

export default function VolumeDetailPage({ params }: { params: Promise<{ volumeId: string }> }) {
  const { volumeId } = use(params);
  const [editOpen, setEditOpen] = useState(false);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const { data: volume, isLoading, error } = useVolume(volumeId);
  const updateVolume = useUpdateVolume();
  const syncVolume = useSyncVolume();
  const archiveVolume = useArchiveVolume();
  usePageTitle(volume ? volume.name : null, "Volumes");

  if (isLoading) {
    return <VolumeDetailSkeleton />;
  }

  if (error || !volume) {
    return (
      <ResourceNotFound
        title="Volume not found"
        description="The requested volume is not available in the current organization."
        backHref="/volumes"
        backLabel="Back to Volumes"
        resourceId={volumeId}
      />
    );
  }

  const isReadOnly = isReadOnlyStatus(volume.status);
  const canSync = volume.source_type !== "manual" && volume.status === "active";

  return (
    <div className="container mx-auto space-y-6 p-6">
      <div className="flex items-center justify-between">
        <Link href="/volumes">
          <Button variant="outline">
            <ArrowLeft className="h-4 w-4" />
            Volumes
          </Button>
        </Link>
        <div className="flex items-center gap-2">
          <Button variant="outline" onClick={() => setEditOpen(true)} disabled={isReadOnly}>
            <Pencil className="h-4 w-4" />
            Edit
          </Button>
          {!isReadOnly && (
            <Button variant="outline" onClick={() => setArchiveOpen(true)}>
              <Archive className="h-4 w-4" />
              Archive
            </Button>
          )}
          {canSync && (
            <Button
              variant="outline"
              onClick={() => syncVolume.mutate(volume.id)}
              disabled={
                syncVolume.isPending ||
                volume.sync_status === "pending" ||
                volume.sync_status === "syncing"
              }
            >
              <RefreshCw className="h-4 w-4" />
              Sync
            </Button>
          )}
        </div>
      </div>

      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center bg-primary/10">
            <HardDrive className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <h1 className={`truncate text-2xl font-bold ${getEntityNameClassName(volume.status)}`}>
              {volume.name}
            </h1>
            <div className="mt-1 flex items-center gap-1.5 text-sm text-muted-foreground">
              <span className="truncate font-mono">{volume.id}</span>
              <CopyButton value={volume.id} />
            </div>
          </div>
        </div>
        <Badge variant={getEntityStatusBadgeVariant(volume.status)}>{volume.status}</Badge>
      </div>

      <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
        <Card>
          <CardHeader>
            <CardTitle>Overview</CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">
              {volume.description || "No description"}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Source</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div>
              <div className="font-medium">Provider</div>
              <div className="flex items-center gap-1.5 text-muted-foreground">
                {volume.source.provider === "github" ? (
                  <Github className="h-4 w-4" />
                ) : volume.source.provider === "git" ? (
                  <GitBranch className="h-4 w-4" />
                ) : (
                  <HardDrive className="h-4 w-4" />
                )}
                <span>{sourceLabel(volume)}</span>
              </div>
            </div>
            {volume.source.provider !== "manual" && (
              <>
                <div>
                  <div className="font-medium">Branch</div>
                  <div className="text-muted-foreground">{volume.source.branch}</div>
                </div>
                {volume.source.root_folder && (
                  <div>
                    <div className="font-medium">Root Folder</div>
                    <div className="font-mono text-muted-foreground">
                      {volume.source.root_folder}
                    </div>
                  </div>
                )}
                <div>
                  <div className="font-medium">Resync</div>
                  <div className="text-muted-foreground">{formatSyncInterval(volume)}</div>
                </div>
              </>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Activity</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div>
              <div className="font-medium">Created</div>
              <div className="text-muted-foreground">{formatDate(volume.created_at)}</div>
            </div>
            <div>
              <div className="font-medium">Updated</div>
              <div className="text-muted-foreground">{formatRelativeTime(volume.updated_at)}</div>
            </div>
            <div>
              <div className="font-medium">Sync</div>
              <div className="text-muted-foreground">{formatSyncStatus(volume)}</div>
            </div>
            {volume.last_sync_error && (
              <div className="flex gap-2 text-destructive">
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                <span>{volume.last_sync_error}</span>
              </div>
            )}
            {volume.archived_at && (
              <div>
                <div className="font-medium">Archived</div>
                <div className="text-muted-foreground">{formatDate(volume.archived_at)}</div>
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      <VolumeFormDialog
        mode="edit"
        open={editOpen}
        volume={volume}
        isPending={updateVolume.isPending}
        onOpenChange={setEditOpen}
        onSubmit={async (request) => {
          await updateVolume.mutateAsync({
            volumeId: volume.id,
            data: request as UpdateVolumeRequest,
          });
        }}
      />
      <ArchiveVolumeDialog
        open={archiveOpen}
        volume={volume}
        isPending={archiveVolume.isPending}
        onOpenChange={setArchiveOpen}
        onArchive={() => archiveVolume.mutateAsync(volume.id)}
      />
    </div>
  );
}

function sourceLabel(volume: Volume) {
  if (volume.source.provider === "github") {
    return volume.source.repository;
  }
  if (volume.source.provider === "git") {
    return volume.source.url;
  }
  return "Manual";
}

function formatSyncStatus(volume: Volume) {
  if (volume.source_type === "manual") {
    return "Manual";
  }
  if (volume.last_synced_at) {
    return `${volume.sync_status} · last synced ${formatRelativeTime(volume.last_synced_at)}`;
  }
  return volume.sync_status;
}

function formatSyncInterval(volume: Volume) {
  const interval =
    volume.source.provider === "github" || volume.source.provider === "git"
      ? volume.source.sync_interval_secs
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

function VolumeDetailSkeleton() {
  return (
    <div className="container mx-auto space-y-6 p-6">
      <Skeleton className="h-8 w-28" />
      <div className="flex items-start gap-3">
        <Skeleton className="h-10 w-10" />
        <div className="space-y-2">
          <Skeleton className="h-8 w-64" />
          <Skeleton className="h-4 w-80" />
        </div>
      </div>
      <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
        <Skeleton className="h-40" />
        <Skeleton className="h-40" />
      </div>
    </div>
  );
}
