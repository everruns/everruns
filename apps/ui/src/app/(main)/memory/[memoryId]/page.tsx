"use client";

import { use, useState } from "react";
import { AlertCircle, Archive, Brain, GitBranch, HardDrive, Pencil, RefreshCw } from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ArchiveMemoryDialog } from "@/components/memory/archive-memory-dialog";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import { useArchiveMemory, usePageTitle, useSyncMemory, useUpdateMemory, useMemory } from "@/hooks";
import type { UpdateMemoryRequest, Memory } from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatDate, formatRelativeTime } from "@/lib/formatting";

export default function MemoryDetailPage({ params }: { params: Promise<{ memoryId: string }> }) {
  const { memoryId } = use(params);
  const [editOpen, setEditOpen] = useState(false);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const { data: memory, isLoading, error } = useMemory(memoryId);
  const updateMemory = useUpdateMemory();
  const syncMemory = useSyncMemory();
  const archiveMemory = useArchiveMemory();
  usePageTitle(memory ? memory.name : null, "Memory");

  if (isLoading) {
    return <MemoryDetailSkeleton />;
  }

  if (error || !memory) {
    return (
      <ResourceNotFound
        title="Memory not found"
        description="The requested memory is not available in the current organization."
        backHref="/memory"
        backLabel="Back to Memory"
        resourceId={memoryId}
      />
    );
  }

  const isReadOnly = isReadOnlyStatus(memory.status);
  const canSync = memory.source_type !== "manual" && memory.status === "active";

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Memory", href: "/memory" }, { label: memory.name }]} />

      <PageMasthead
        icon={<Brain />}
        entityId={memory.id}
        title={<span className={getEntityNameClassName(memory.status)}>{memory.name}</span>}
        badges={
          <>
            <Badge variant={getEntityStatusBadgeVariant(memory.status)}>{memory.status}</Badge>
            {memory.is_readonly && <Badge variant="secondary">Read-only</Badge>}
          </>
        }
        meta={
          <>
            <span>
              Source <span className="text-foreground">{sourceLabel(memory)}</span>
            </span>
          </>
        }
        actions={
          <>
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
                onClick={() => syncMemory.mutate(memory.id)}
                disabled={
                  syncMemory.isPending ||
                  memory.sync_status === "pending" ||
                  memory.sync_status === "syncing"
                }
              >
                <RefreshCw className="h-4 w-4" />
                Sync
              </Button>
            )}
          </>
        }
      />

      <PageColumns>
        <PageMain>
          <Card>
            <CardHeader>
              <CardTitle>Overview</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-sm text-muted-foreground">
                {memory.description || "No description"}
              </p>
            </CardContent>
          </Card>
        </PageMain>

        <PageRail>
          <Card>
            <CardHeader>
              <CardTitle>Source</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              <div>
                <div className="font-medium">Provider</div>
                <div className="flex items-center gap-1.5 text-muted-foreground">
                  {memory.source.provider === "github" ? (
                    <Github className="h-4 w-4" />
                  ) : memory.source.provider === "git" ? (
                    <GitBranch className="h-4 w-4" />
                  ) : (
                    <HardDrive className="h-4 w-4" />
                  )}
                  <span>{sourceLabel(memory)}</span>
                </div>
              </div>
              {memory.source.provider !== "manual" && (
                <>
                  <div>
                    <div className="font-medium">Branch</div>
                    <div className="text-muted-foreground">{memory.source.branch}</div>
                  </div>
                  {memory.source.root_folder && (
                    <div>
                      <div className="font-medium">Root Folder</div>
                      <div className="font-mono text-muted-foreground">
                        {memory.source.root_folder}
                      </div>
                    </div>
                  )}
                  <div>
                    <div className="font-medium">Resync</div>
                    <div className="text-muted-foreground">{formatSyncInterval(memory)}</div>
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
                <div className="text-muted-foreground">{formatDate(memory.created_at)}</div>
              </div>
              <div>
                <div className="font-medium">Updated</div>
                <div className="text-muted-foreground">{formatRelativeTime(memory.updated_at)}</div>
              </div>
              <div>
                <div className="font-medium">Sync</div>
                <div className="text-muted-foreground">{formatSyncStatus(memory)}</div>
              </div>
              {memory.last_sync_error && (
                <div className="flex gap-2 text-destructive">
                  <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                  <span>{memory.last_sync_error}</span>
                </div>
              )}
              {memory.archived_at && (
                <div>
                  <div className="font-medium">Archived</div>
                  <div className="text-muted-foreground">{formatDate(memory.archived_at)}</div>
                </div>
              )}
            </CardContent>
          </Card>
        </PageRail>
      </PageColumns>

      <PageFooter>
        <BackLink href="/memory">Back to Memory</BackLink>
      </PageFooter>

      <MemoryFormDialog
        mode="edit"
        open={editOpen}
        memory={memory}
        isPending={updateMemory.isPending}
        onOpenChange={setEditOpen}
        onSubmit={async (request) => {
          await updateMemory.mutateAsync({
            memoryId: memory.id,
            data: request as UpdateMemoryRequest,
          });
        }}
      />
      <ArchiveMemoryDialog
        open={archiveOpen}
        memory={memory}
        isPending={archiveMemory.isPending}
        onOpenChange={setArchiveOpen}
        onArchive={() => archiveMemory.mutateAsync(memory.id)}
      />
    </PageContainer>
  );
}

function sourceLabel(memory: Memory) {
  if (memory.source.provider === "github") {
    return memory.source.repository;
  }
  if (memory.source.provider === "git") {
    return memory.source.url;
  }
  return "Manual";
}

function formatSyncStatus(memory: Memory) {
  if (memory.source_type === "manual") {
    return "Manual";
  }
  if (memory.last_synced_at) {
    return `${memory.sync_status} · last synced ${formatRelativeTime(memory.last_synced_at)}`;
  }
  return memory.sync_status;
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

function MemoryDetailSkeleton() {
  return (
    <PageContainer>
      <Skeleton className="h-4 w-48" />
      <div className="flex items-start gap-4 border-b pb-4">
        <Skeleton className="size-11" />
        <div className="space-y-2">
          <Skeleton className="h-8 w-64" />
          <Skeleton className="h-4 w-80" />
        </div>
      </div>
      <PageColumns>
        <Skeleton className="h-40" />
        <Skeleton className="h-40" />
      </PageColumns>
    </PageContainer>
  );
}
