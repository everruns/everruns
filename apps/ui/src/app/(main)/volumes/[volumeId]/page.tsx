"use client";

import Link from "next/link";
import { use, useState } from "react";
import { Archive, ArrowLeft, HardDrive, Pencil } from "lucide-react";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ArchiveVolumeDialog } from "@/components/volumes/archive-volume-dialog";
import { VolumeFormDialog } from "@/components/volumes/volume-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import { useArchiveVolume, usePageTitle, useUpdateVolume, useVolume } from "@/hooks";
import type { UpdateVolumeRequest } from "@/lib/api/types";
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
  const archiveVolume = useArchiveVolume();
  usePageTitle(volume ? volume.name : null, "Volumes");

  if (isLoading) {
    return <VolumeDetailSkeleton />;
  }

  if (error || !volume) {
    return (
      <ResourceNotFound
        title="Volume not found"
        description="The requested volume is not available in the current organisation."
        backHref="/volumes"
        backLabel="Back to Volumes"
        resourceId={volumeId}
      />
    );
  }

  const isReadOnly = isReadOnlyStatus(volume.status);

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
