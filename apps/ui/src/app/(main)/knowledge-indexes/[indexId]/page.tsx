"use client";

import Link from "next/link";
import { use, useState } from "react";
import {
  AlertCircle,
  Archive,
  ArrowLeft,
  GitBranch,
  Library,
  Pencil,
  RefreshCw,
} from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ArchiveKnowledgeIndexDialog } from "@/components/knowledge-indexes/archive-knowledge-index-dialog";
import { KnowledgeIndexFormDialog } from "@/components/knowledge-indexes/knowledge-index-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useArchiveKnowledgeIndex,
  useKnowledgeIndex,
  useKnowledgeIndexDocuments,
  usePageTitle,
  useSyncKnowledgeIndex,
  useUpdateKnowledgeIndex,
} from "@/hooks";
import type { KnowledgeIndex, UpdateKnowledgeIndexRequest } from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatDate, formatRelativeTime } from "@/lib/formatting";
import { syncStatusBadgeVariant } from "@/lib/knowledge-index-sync";

function sourceField(index: KnowledgeIndex, key: string): string | null {
  const value = (index.source_config as Record<string, unknown>)?.[key];
  return typeof value === "string" && value.length > 0 ? value : null;
}

export default function KnowledgeIndexDetailPage({
  params,
}: {
  params: Promise<{ indexId: string }>;
}) {
  const { indexId } = use(params);
  const [editOpen, setEditOpen] = useState(false);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const { data: index, isLoading, error } = useKnowledgeIndex(indexId);
  const updateIndex = useUpdateKnowledgeIndex();
  const syncIndex = useSyncKnowledgeIndex();
  const archiveIndex = useArchiveKnowledgeIndex();
  usePageTitle(index ? index.name : null, "Knowledge Indexes");

  if (isLoading) {
    return <DetailSkeleton />;
  }

  if (error || !index) {
    return (
      <ResourceNotFound
        title="Knowledge index not found"
        description="The requested knowledge index is not available in the current organization."
        backHref="/knowledge-indexes"
        backLabel="Back to Knowledge Indexes"
        resourceId={indexId}
      />
    );
  }

  const isReadOnly = isReadOnlyStatus(index.status);
  const canSync = index.status === "active";
  const repository = sourceField(index, "repository");
  const gitUrl = sourceField(index, "url");
  const branch = sourceField(index, "branch");
  const rootFolder = sourceField(index, "root_folder");

  return (
    <div className="container mx-auto space-y-6 p-6">
      <div className="flex items-center justify-between">
        <Link href="/knowledge-indexes">
          <Button variant="outline">
            <ArrowLeft className="h-4 w-4" />
            Knowledge Indexes
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
              variant="accent"
              onClick={() => syncIndex.mutate(index.id)}
              disabled={
                syncIndex.isPending ||
                index.sync_status === "pending" ||
                index.sync_status === "syncing"
              }
            >
              <RefreshCw className="h-4 w-4" />
              Sync now
            </Button>
          )}
        </div>
      </div>

      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center bg-primary/10">
            <Library className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <h1 className={`truncate text-2xl font-bold ${getEntityNameClassName(index.status)}`}>
              {index.name}
            </h1>
            <div className="mt-1 flex items-center gap-1.5 text-sm text-muted-foreground">
              <span className="truncate font-mono">{index.id}</span>
              <CopyButton value={index.id} />
            </div>
          </div>
        </div>
        <Badge variant={getEntityStatusBadgeVariant(index.status)}>{index.status}</Badge>
      </div>

      <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
        <Card>
          <CardHeader>
            <CardTitle>Overview</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <p className="text-muted-foreground">{index.description || "No description"}</p>
            <div>
              <div className="font-medium">Embedding model</div>
              <div className="font-mono text-muted-foreground">{index.embedding_model_id}</div>
            </div>
            {typeof index.vector_dim === "number" && (
              <div>
                <div className="font-medium">Vector dimension</div>
                <div className="text-muted-foreground">{index.vector_dim}</div>
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Source</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div>
              <div className="font-medium">Type</div>
              <div className="flex items-center gap-1.5 text-muted-foreground">
                {index.source_type === "github" ? (
                  <Github className="h-4 w-4" />
                ) : (
                  <GitBranch className="h-4 w-4" />
                )}
                <span>{index.source_type}</span>
              </div>
            </div>
            {repository && (
              <div>
                <div className="font-medium">Repository</div>
                <div className="font-mono text-muted-foreground">{repository}</div>
              </div>
            )}
            {gitUrl && (
              <div>
                <div className="font-medium">Git URL</div>
                <div className="break-all font-mono text-muted-foreground">{gitUrl}</div>
              </div>
            )}
            {branch && (
              <div>
                <div className="font-medium">Branch</div>
                <div className="text-muted-foreground">{branch}</div>
              </div>
            )}
            {rootFolder && (
              <div>
                <div className="font-medium">Root Folder</div>
                <div className="font-mono text-muted-foreground">{rootFolder}</div>
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Sync status</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
            <div>
              <div className="font-medium">Status</div>
              <Badge variant={syncStatusBadgeVariant(index.sync_status)}>{index.sync_status}</Badge>
            </div>
            <div>
              <div className="font-medium">Last synced</div>
              <div className="text-muted-foreground">
                {index.last_synced_at ? formatRelativeTime(index.last_synced_at) : "Never"}
              </div>
            </div>
            <div>
              <div className="font-medium">Created</div>
              <div className="text-muted-foreground">{formatDate(index.created_at)}</div>
            </div>
            <div>
              <div className="font-medium">Updated</div>
              <div className="text-muted-foreground">{formatRelativeTime(index.updated_at)}</div>
            </div>
          </div>
          {index.last_sync_error && (
            <div className="flex gap-2 text-destructive">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{index.last_sync_error}</span>
            </div>
          )}
          {canSync && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => syncIndex.mutate(index.id)}
              disabled={
                syncIndex.isPending ||
                index.sync_status === "pending" ||
                index.sync_status === "syncing"
              }
            >
              <RefreshCw className="h-4 w-4" />
              Sync now
            </Button>
          )}
        </CardContent>
      </Card>

      <DocumentsCard indexId={index.id} />

      <KnowledgeIndexFormDialog
        mode="edit"
        open={editOpen}
        index={index}
        isPending={updateIndex.isPending}
        onOpenChange={setEditOpen}
        onSubmit={async (request) => {
          await updateIndex.mutateAsync({
            indexId: index.id,
            data: request as UpdateKnowledgeIndexRequest,
          });
        }}
      />
      <ArchiveKnowledgeIndexDialog
        open={archiveOpen}
        index={index}
        isPending={archiveIndex.isPending}
        onOpenChange={setArchiveOpen}
        onArchive={() => archiveIndex.mutateAsync(index.id)}
      />
    </div>
  );
}

function DocumentsCard({ indexId }: { indexId: string }) {
  const { data: documents, isLoading, error } = useKnowledgeIndexDocuments(indexId);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Documents</CardTitle>
      </CardHeader>
      <CardContent>
        {isLoading && <Skeleton className="h-24 w-full" />}
        {error && <p className="text-sm text-destructive">Failed to load documents.</p>}
        {!isLoading && !error && (documents?.length ?? 0) === 0 && (
          <p className="text-sm text-muted-foreground">
            No documents indexed yet. Run a sync to populate this index.
          </p>
        )}
        {!isLoading && !error && (documents?.length ?? 0) > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b text-left text-xs text-muted-foreground">
                  <th className="py-2 pr-4 font-medium">Document</th>
                  <th className="py-2 pr-4 font-medium">Chunks</th>
                  <th className="py-2 font-medium">Last seen</th>
                </tr>
              </thead>
              <tbody>
                {documents!.map((doc) => (
                  <tr key={doc.id} className="border-b last:border-0">
                    <td className="py-2 pr-4">
                      <div className="font-medium">{doc.title || doc.source_uri}</div>
                      {doc.title && (
                        <div className="break-all font-mono text-xs text-muted-foreground">
                          {doc.source_uri}
                        </div>
                      )}
                    </td>
                    <td className="py-2 pr-4 text-muted-foreground">{doc.chunk_count}</td>
                    <td className="py-2 text-muted-foreground">
                      {doc.last_seen_at ? formatRelativeTime(doc.last_seen_at) : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function DetailSkeleton() {
  return (
    <div className="container mx-auto space-y-6 p-6">
      <Skeleton className="h-8 w-44" />
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
      <Skeleton className="h-32" />
    </div>
  );
}
