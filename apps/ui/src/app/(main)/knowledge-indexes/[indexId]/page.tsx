"use client";

import Link from "next/link";
import { use, useMemo, useState } from "react";
import { Archive, GitBranch, Library, Pencil, RefreshCw } from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ArchiveKnowledgeIndexDialog } from "@/components/knowledge-indexes/archive-knowledge-index-dialog";
import {
  KnowledgeIndexDiagnosticBadge,
  KnowledgeIndexDiagnosticNotice,
} from "@/components/knowledge-indexes/knowledge-index-diagnostic";
import { KnowledgeIndexFormDialog } from "@/components/knowledge-indexes/knowledge-index-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  StatCard,
  StatGrid,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
  BackLink,
} from "@/components/layout";
import {
  useArchiveKnowledgeIndex,
  useKnowledgeIndex,
  useKnowledgeIndexDocuments,
  usePageTitle,
  useSyncKnowledgeIndex,
  useUpdateKnowledgeIndex,
} from "@/hooks";
import { useModels } from "@/hooks/use-providers";
import type {
  KnowledgeIndex,
  KnowledgeIndexDocument,
  UpdateKnowledgeIndexRequest,
} from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatDate, formatRelativeTime } from "@/lib/formatting";
import {
  getKnowledgeIndexDiagnostic,
  knowledgeIndexDiagnosticBlocksSync,
  knowledgeIndexSyncActionLabel,
  type KnowledgeIndexDiagnostic,
} from "@/lib/knowledge-index-diagnostics";

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
  const {
    data: documents,
    isLoading: documentsLoading,
    error: documentsError,
  } = useKnowledgeIndexDocuments(indexId);
  const updateIndex = useUpdateKnowledgeIndex();
  const syncIndex = useSyncKnowledgeIndex();
  const archiveIndex = useArchiveKnowledgeIndex();
  const { data: models, isLoading: modelsLoading, error: modelsError } = useModels();
  usePageTitle(index ? index.name : null, "Knowledge Indexes");

  const totalChunks = useMemo(
    () => (documents ?? []).reduce((sum, doc) => sum + (doc.chunk_count ?? 0), 0),
    [documents],
  );

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
  const diagnostic = getKnowledgeIndexDiagnostic(index, {
    models,
    modelsReady: !modelsLoading && !modelsError,
  });
  const canSync = index.status === "active" && !knowledgeIndexDiagnosticBlocksSync(diagnostic);
  const repository = sourceField(index, "repository");
  const gitUrl = sourceField(index, "url");
  const branch = sourceField(index, "branch");
  const rootFolder = sourceField(index, "root_folder");
  const documentCount = documents?.length ?? 0;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Knowledge Indexes", href: "/knowledge-indexes" }, { label: index.name }]}
      />

      <PageMasthead
        icon={<Library />}
        entityId={index.id}
        title={<span className={getEntityNameClassName(index.status)}>{index.name}</span>}
        badges={
          <>
            <Badge variant={getEntityStatusBadgeVariant(index.status)}>{index.status}</Badge>
            <KnowledgeIndexDiagnosticBadge diagnostic={diagnostic} />
          </>
        }
        description={index.description || undefined}
        meta={
          <>
            <span>
              Embedding <span className="font-mono text-primary">{index.embedding_model_id}</span>
            </span>
            <span>
              Source{" "}
              <span className="inline-flex items-center gap-1 text-primary">
                {index.source_type === "github" ? (
                  <Github className="size-3.5" />
                ) : (
                  <GitBranch className="size-3.5" />
                )}
                {index.source_type}
              </span>
            </span>
            <span>
              Last synced{" "}
              <span className="text-foreground">
                {index.last_synced_at ? formatRelativeTime(index.last_synced_at) : "Never"}
              </span>
            </span>
          </>
        }
        actions={
          <>
            <Button variant="outline" onClick={() => setEditOpen(true)} disabled={isReadOnly}>
              <Pencil className="size-4" />
              Edit
            </Button>
            {!isReadOnly && (
              <Button variant="outline" onClick={() => setArchiveOpen(true)}>
                <Archive className="size-4" />
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
                <RefreshCw className="size-4" />
                {knowledgeIndexSyncActionLabel(diagnostic)}
              </Button>
            )}
          </>
        }
      />

      <KnowledgeIndexDiagnosticNotice
        diagnostic={diagnostic}
        action={
          <DiagnosticAction
            diagnostic={diagnostic}
            index={index}
            canSync={canSync}
            isSyncing={syncIndex.isPending}
            onEdit={() => setEditOpen(true)}
            onSync={() => syncIndex.mutate(index.id)}
          />
        }
      />

      <PageControlStrip>
        <StatGrid className="lg:grid-cols-3">
          <StatCard
            label="Documents"
            value={documentsLoading ? "—" : documentCount}
            hint="indexed"
          />
          <StatCard label="Chunks" value={documentsLoading ? "—" : totalChunks} hint="embedded" />
          <StatCard
            label="Vector dim"
            value={typeof index.vector_dim === "number" ? index.vector_dim : "—"}
          />
        </StatGrid>
      </PageControlStrip>

      <PageColumns>
        <PageMain>
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

          <DocumentsCard
            documents={documents}
            isLoading={documentsLoading}
            error={documentsError}
          />
        </PageMain>

        <PageRail>
          <RailSection label="Source">
            <div className="space-y-3 text-sm">
              <div>
                <div className="font-medium">Type</div>
                <div className="flex items-center gap-1.5 text-muted-foreground">
                  {index.source_type === "github" ? (
                    <Github className="size-4" />
                  ) : (
                    <GitBranch className="size-4" />
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
            </div>
          </RailSection>

          <RailSection label="Sync status">
            <div className="space-y-3 text-sm">
              <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
                <div>
                  <div className="font-medium">Status</div>
                  <KnowledgeIndexDiagnosticBadge diagnostic={diagnostic} />
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
                  <div className="text-muted-foreground">
                    {formatRelativeTime(index.updated_at)}
                  </div>
                </div>
              </div>
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
                  <RefreshCw className="size-4" />
                  {knowledgeIndexSyncActionLabel(diagnostic)}
                </Button>
              )}
            </div>
          </RailSection>
        </PageRail>
      </PageColumns>

      <PageFooter>
        <BackLink href="/knowledge-indexes">Back to Knowledge Indexes</BackLink>
      </PageFooter>

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
    </PageContainer>
  );
}

function DiagnosticAction({
  diagnostic,
  index,
  canSync,
  isSyncing,
  onEdit,
  onSync,
}: {
  diagnostic: KnowledgeIndexDiagnostic;
  index: KnowledgeIndex;
  canSync: boolean;
  isSyncing: boolean;
  onEdit: () => void;
  onSync: () => void;
}) {
  if (diagnostic.action === "configure_model") {
    return (
      <Button size="sm" variant="outline" onClick={onEdit} disabled={index.status !== "active"}>
        Configure embedding model
      </Button>
    );
  }
  if (diagnostic.action === "configure_provider" && diagnostic.providerId) {
    return (
      <Button
        size="sm"
        variant="outline"
        render={<Link href={`/settings/providers/${diagnostic.providerId}`} />}
      >
        Configure provider
      </Button>
    );
  }
  if (
    diagnostic.action === "retry_sync" ||
    diagnostic.action === "start_sync" ||
    diagnostic.action === "sync_changes" ||
    diagnostic.action === "sync_again"
  ) {
    return (
      <Button size="sm" variant="outline" onClick={onSync} disabled={!canSync || isSyncing}>
        <RefreshCw className="size-4" />
        {knowledgeIndexSyncActionLabel(diagnostic)}
      </Button>
    );
  }
  return null;
}

function DocumentsCard({
  documents,
  isLoading,
  error,
}: {
  documents: KnowledgeIndexDocument[] | undefined;
  isLoading: boolean;
  error: unknown;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Documents</CardTitle>
      </CardHeader>
      <CardContent>
        {isLoading && <Skeleton className="h-24 w-full" />}
        {!!error && <p className="text-sm text-destructive">Failed to load documents.</p>}
        {!isLoading && !error && (documents?.length ?? 0) === 0 && (
          <p className="text-sm text-muted-foreground">
            No documents indexed yet. Run a sync to populate this index.
          </p>
        )}
        {!isLoading && !error && (documents?.length ?? 0) > 0 && (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Document</TableHead>
                <TableHead>Chunks</TableHead>
                <TableHead>Last seen</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {documents!.map((doc) => (
                <TableRow key={doc.id}>
                  <TableCell>
                    <div className="font-medium">{doc.title || doc.source_uri}</div>
                    {doc.title && (
                      <div className="break-all font-mono text-xs text-muted-foreground">
                        {doc.source_uri}
                      </div>
                    )}
                  </TableCell>
                  <TableCell className="text-muted-foreground">{doc.chunk_count}</TableCell>
                  <TableCell className="text-muted-foreground">
                    {doc.last_seen_at ? formatRelativeTime(doc.last_seen_at) : "—"}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
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
