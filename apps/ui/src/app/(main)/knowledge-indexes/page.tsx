"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { Archive, FolderOpen, GitBranch, Library, Pencil, Plus, RefreshCw } from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ArchiveKnowledgeIndexDialog } from "@/components/knowledge-indexes/archive-knowledge-index-dialog";
import { KnowledgeIndexDiagnosticBadge } from "@/components/knowledge-indexes/knowledge-index-diagnostic";
import { KnowledgeIndexFormDialog } from "@/components/knowledge-indexes/knowledge-index-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { SearchInput } from "@/components/ui/search-input";
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
  SectionTabs,
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
} from "@/components/layout";
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
  isArchivedStatus,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { formatRelativeTime, pluralize } from "@/lib/formatting";
import {
  getKnowledgeIndexDiagnostic,
  knowledgeIndexDiagnosticBlocksSync,
  knowledgeIndexSyncActionLabel,
  type KnowledgeIndexDiagnostic,
} from "@/lib/knowledge-index-diagnostics";
import { cn } from "@/lib/utils";
import { useModels } from "@/hooks/use-providers";

type StatusTab = "all" | "active" | "archived";

const statusItems = [
  { value: "all" as const, label: "All" },
  { value: "active" as const, label: "Active" },
  { value: "archived" as const, label: "Archived" },
];

export default function KnowledgeIndexesPage() {
  usePageTitle("Knowledge Indexes");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [search, setSearch] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [editingIndex, setEditingIndex] = useState<KnowledgeIndex | null>(null);
  const [archivingIndex, setArchivingIndex] = useState<KnowledgeIndex | null>(null);
  // Fetch the full set (including archived) so the facet rail can show accurate counts;
  // the active/archived/all split is applied client-side from the same response.
  const {
    data: indexes,
    isLoading,
    error,
  } = useKnowledgeIndexes({ includeArchived: true, search });
  const createIndex = useCreateKnowledgeIndex();
  const updateIndex = useUpdateKnowledgeIndex();
  const syncIndex = useSyncKnowledgeIndex();
  const archiveIndex = useArchiveKnowledgeIndex();
  const { data: models, isLoading: modelsLoading, error: modelsError } = useModels();

  const counts = useMemo(() => {
    const list = indexes ?? [];
    const archived = list.filter((i) => isArchivedStatus(i.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [indexes]);

  const sourceFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const index of indexes ?? []) {
      tally.set(index.source_type, (tally.get(index.source_type) ?? 0) + 1);
    }
    return [...tally.entries()].sort((a, b) => b[1] - a[1]);
  }, [indexes]);

  const syncFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const index of indexes ?? []) {
      tally.set(index.sync_status, (tally.get(index.sync_status) ?? 0) + 1);
    }
    return [...tally.entries()].sort((a, b) => b[1] - a[1]);
  }, [indexes]);

  const filteredIndexes = useMemo(() => {
    return (indexes ?? []).filter((index) => {
      if (statusTab === "active" && isArchivedStatus(index.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(index.status)) return false;
      return true;
    });
  }, [indexes, statusTab]);

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Knowledge Indexes" }]} />

      <PageMasthead
        icon={<Library />}
        title="Knowledge Indexes"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Synced document collections that power retrieval — sources, embeddings, and sync state."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <Button variant="accent" onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" />
            New index
          </Button>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search knowledge indexes…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          aria-label="Search knowledge indexes"
          containerClassName="w-64 max-w-full"
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
            data={filteredIndexes}
            errorMessagePrefix="Failed to load knowledge indexes"
            skeletonCount={6}
            emptyState={
              <EmptyState
                icon={<Library />}
                title={
                  search.trim() || statusTab !== "active"
                    ? "No knowledge indexes match your filters."
                    : "No knowledge indexes yet"
                }
                action={
                  !search.trim() &&
                  statusTab === "active" && (
                    <Button variant="accent" onClick={() => setCreateOpen(true)}>
                      <Plus className="size-4" />
                      New index
                    </Button>
                  )
                }
              />
            }
          >
            {(items) => (
              <div className="min-w-0 border">
                <Table aria-label="Knowledge indexes" className="xl:min-w-[960px]">
                  <TableHeader className="hidden xl:table-header-group">
                    <TableRow>
                      <TableHead>Name</TableHead>
                      <TableHead>Source</TableHead>
                      <TableHead>Index state</TableHead>
                      <TableHead>Last synced</TableHead>
                      <TableHead>Updated</TableHead>
                      <TableHead className="text-right">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {items.map((index) => (
                      <KnowledgeIndexRow
                        key={index.id}
                        index={index}
                        onEdit={setEditingIndex}
                        onArchive={setArchivingIndex}
                        onSync={(candidate) => syncIndex.mutate(candidate.id)}
                        isSyncing={syncIndex.isPending}
                        diagnostic={getKnowledgeIndexDiagnostic(index, {
                          models,
                          modelsReady: !modelsLoading && !modelsError,
                        })}
                      />
                    ))}
                  </TableBody>
                </Table>
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
                      ) : (
                        <GitBranch className="size-3.5" />
                      )}
                      {source}
                    </span>
                    <span>{count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}

          {syncFacets.length > 0 && (
            <RailSection label="Sync status">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {syncFacets.map(([syncStatus, count]) => (
                  <div key={syncStatus} className="flex items-center justify-between">
                    <span>{syncStatus}</span>
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
          Showing {filteredIndexes.length} of {counts.all}{" "}
          {pluralize(counts.all, "index", "indexes")}
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
    </PageContainer>
  );
}

function KnowledgeIndexRow({
  index,
  onEdit,
  onArchive,
  onSync,
  isSyncing,
  diagnostic,
}: {
  index: KnowledgeIndex;
  onEdit: (index: KnowledgeIndex) => void;
  onArchive: (index: KnowledgeIndex) => void;
  onSync: (index: KnowledgeIndex) => void;
  isSyncing: boolean;
  diagnostic: KnowledgeIndexDiagnostic;
}) {
  const isReadOnly = isReadOnlyStatus(index.status);
  const canSync = index.status === "active" && !knowledgeIndexDiagnosticBlocksSync(diagnostic);

  return (
    <TableRow
      data-slot="responsive-table-row"
      className="grid grid-cols-2 gap-x-4 gap-y-4 p-4 hover:bg-transparent sm:grid-cols-4 xl:table-row xl:p-0 xl:hover:bg-muted/50"
    >
      <TableCell className="col-span-2 block min-w-0 whitespace-normal p-0 sm:col-span-4 xl:table-cell xl:p-1.5">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <div className="font-medium">
            <EntityIdentity value={index.id} labelClassName={getEntityNameClassName(index.status)}>
              <Link href={`/knowledge-indexes/${index.id}`} className="hover:underline">
                {index.name}
              </Link>
            </EntityIdentity>
          </div>
          <Badge variant={getEntityStatusBadgeVariant(index.status)}>{index.status}</Badge>
        </div>
        {index.description && (
          <div className="mt-0.5 line-clamp-1 text-xs text-muted-foreground">
            {index.description}
          </div>
        )}
      </TableCell>
      <TableCell className="col-span-2 block min-w-0 whitespace-normal p-0 sm:col-span-1 xl:table-cell xl:p-1.5">
        <span className="mb-1 block font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground xl:hidden">
          Source
        </span>
        <span className="inline-flex items-center gap-1.5 text-muted-foreground">
          {index.source_type === "github" ? (
            <Github className="size-3.5" />
          ) : (
            <GitBranch className="size-3.5" />
          )}
          {index.source_type}
        </span>
      </TableCell>
      <TableCell className="col-span-2 block min-w-0 whitespace-normal p-0 sm:col-span-3 xl:table-cell xl:p-1.5">
        <span className="mb-1 block font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground xl:hidden">
          Index state
        </span>
        <div className="min-w-0 space-y-1.5 whitespace-normal xl:w-64">
          <KnowledgeIndexDiagnosticBadge diagnostic={diagnostic} />
          <p className="text-xs text-muted-foreground">{diagnostic.description}</p>
          {diagnostic.failureReason && (
            <p className="line-clamp-2 text-xs text-destructive">
              <span className="font-medium">Latest failure:</span> {diagnostic.failureReason}
            </p>
          )}
          {diagnostic.action === "configure_model" && !isReadOnly && (
            <button
              type="button"
              className="text-xs font-medium text-primary hover:underline"
              onClick={() => onEdit(index)}
            >
              Configure embedding model
            </button>
          )}
          {diagnostic.action === "configure_provider" && diagnostic.providerId && (
            <Link
              href={`/settings/providers/${diagnostic.providerId}`}
              className="block text-xs font-medium text-primary hover:underline"
            >
              Configure provider
            </Link>
          )}
        </div>
      </TableCell>
      <TableCell className="col-span-1 block whitespace-normal p-0 text-muted-foreground xl:table-cell xl:p-1.5">
        <span className="mb-1 block font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground xl:hidden">
          Last synced
        </span>
        {index.last_synced_at ? formatRelativeTime(index.last_synced_at) : "Never"}
      </TableCell>
      <TableCell className="col-span-1 block whitespace-normal p-0 text-muted-foreground xl:table-cell xl:p-1.5">
        <span className="mb-1 block font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground xl:hidden">
          Updated
        </span>
        {formatRelativeTime(index.updated_at)}
      </TableCell>
      <TableCell className="col-span-2 block whitespace-normal p-0 sm:col-span-4 xl:table-cell xl:p-1.5">
        <span className="mb-1 block font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground xl:hidden">
          Actions
        </span>
        <div className="flex flex-wrap items-center justify-start gap-2 xl:justify-end">
          {canSync && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onSync(index)}
              disabled={
                isSyncing || index.sync_status === "pending" || index.sync_status === "syncing"
              }
            >
              <RefreshCw className="size-4" />
              {knowledgeIndexSyncActionLabel(diagnostic)}
            </Button>
          )}
          <Link href={`/knowledge-indexes/${index.id}`}>
            <Button variant="outline" size="sm">
              <FolderOpen className="size-4" />
              Open
            </Button>
          </Link>
          <Button variant="outline" size="sm" onClick={() => onEdit(index)} disabled={isReadOnly}>
            <Pencil className="size-4" />
            Edit
          </Button>
          {!isReadOnly && (
            <Button variant="outline" size="sm" onClick={() => onArchive(index)}>
              <Archive className="size-4" />
              Archive
            </Button>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}
