"use client";

// One memory store: its typed entries grouped by kind (open blocked items and
// dreaming proposals first) and the history every change leaves. Entries are
// read-only here; agents record them with `remember`, and people can edit
// the Markdown files directly.

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";
import { MemoryIcon } from "@/components/icons/facet-icons";
import { use, useMemo, useState } from "react";
import { AlertCircle, ExternalLink, Pencil } from "lucide-react";
import { EntityActionsMenu } from "@/components/entity-actions/entity-actions-menu";
import { useOrg } from "@/providers/org-provider";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ArchiveMemoryDialog } from "@/components/memory/archive-memory-dialog";
import { MemoryFormDialog } from "@/components/memory/memory-form-dialog";
import {
  groupMemoryEntries,
  memoryStoreScopeLabel,
  memoryStoreTitle,
} from "@/components/memory/memory-store-labels";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageMain,
  PageFooter,
  BackLink,
  SectionTabs,
  StatCard,
  StatGrid,
  EmptyState,
} from "@/components/layout";
import {
  useArchiveMemory,
  useMemoryEntries,
  useMemoryHistory,
  useMemoryStores,
  usePageTitle,
  useUpdateMemory,
} from "@/hooks";
import type { UpdateMemoryRequest } from "@/lib/api/types";
import type { MemoryEntry, MemoryFileVersion } from "@/lib/api/memory";
import { formatRelativeTime } from "@/lib/formatting";

type Tab = "entries" | "history";

export default function MemoryDetailPage({ params }: { params: Promise<{ memoryId: string }> }) {
  const ui = useUiTranslations();
  const { memoryId } = use(params);
  const [tab, setTab] = useState<Tab>("entries");
  const [editOpen, setEditOpen] = useState(false);
  const [archiveOpen, setArchiveOpen] = useState(false);
  const { data: stores, isLoading, error } = useMemoryStores();
  const store = stores?.find((candidate) => candidate.id === memoryId);
  const updateMemory = useUpdateMemory();
  const archiveMemory = useArchiveMemory();
  const { currentOrg } = useOrg();
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  usePageTitle(store ? memoryStoreTitle(store) : null, ui("Memory"));

  if (isLoading) {
    return <MemoryDetailSkeleton />;
  }

  if (error || !store) {
    return (
      <ResourceNotFound
        title={ui("Memory not found")}
        description={ui("The requested memory is not available in the current organization.")}
        backHref="/memory"
        backLabel="Back to Memory"
        resourceId={memoryId}
      />
    );
  }

  const isNamed = store.scope === "org" && store.store !== "org";

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Memory"), href: "/memory" }, { label: memoryStoreTitle(store) }]}
      />

      <PageMasthead
        icon={<MemoryIcon />}
        entityId={store.id}
        title={memoryStoreTitle(store)}
        description={store.description || ui(store.about)}
        badges={
          <>
            <Badge variant="secondary">{ui(memoryStoreScopeLabel(store))}</Badge>
            {store.is_readonly && <Badge variant="outline">{ui("Read-only")}</Badge>}
          </>
        }
        meta={
          <span>
            {ui("Agents read it at")}{" "}
            <span className="font-mono text-foreground">{store.mount_path}</span>
          </span>
        }
        actions={
          isNamed && (
            <>
              <Button variant="outline" onClick={() => setEditOpen(true)} disabled={!canManage}>
                <Pencil className="h-4 w-4" />
                {ui("Edit")}
              </Button>
              <EntityActionsMenu
                entityRef={store.id}
                kind="memory"
                entityName={store.name}
                permissions={{ manage: canManage }}
                archive={{ onSelect: () => setArchiveOpen(true) }}
              />
            </>
          )
        }
      />

      <StatGrid>
        <StatCard label={ui("Entries")} value={store.entry_count} />
        <StatCard label={ui("Open blocked items")} value={store.blocked_count} />
        <StatCard label={ui("Proposals to review")} value={store.proposal_count} />
        <StatCard label={ui("Files")} value={store.file_count} />
      </StatGrid>

      <SectionTabs
        value={tab}
        onValueChange={(value) => setTab(value as Tab)}
        items={[
          { value: "entries", label: ui("Entries") },
          { value: "history", label: ui("History") },
        ]}
      />

      <PageMain>
        {tab === "entries" ? (
          <EntriesTab memoryId={store.id} />
        ) : (
          <HistoryTab memoryId={store.id} />
        )}
      </PageMain>

      <PageFooter>
        <BackLink href="/memory">{ui("Back to Memory")}</BackLink>
      </PageFooter>

      {isNamed && (
        <>
          <MemoryFormDialog
            writable={canManage}
            mode="edit"
            open={editOpen}
            memory={store}
            isPending={updateMemory.isPending}
            onOpenChange={setEditOpen}
            onSubmit={async (request) => {
              await updateMemory.mutateAsync({
                memoryId: store.id,
                data: request as UpdateMemoryRequest,
              });
            }}
          />
          <ArchiveMemoryDialog
            open={archiveOpen}
            memory={store}
            isPending={archiveMemory.isPending}
            onOpenChange={setArchiveOpen}
            onArchive={() => archiveMemory.mutateAsync(store.id)}
          />
        </>
      )}
    </PageContainer>
  );
}

function EntriesTab({ memoryId }: { memoryId: string }) {
  const ui = useUiTranslations();
  const { data, isLoading, error } = useMemoryEntries(memoryId);
  const sections = useMemo(() => groupMemoryEntries(data?.entries ?? []), [data]);

  return (
    <div className="space-y-8">
      {data && data.invalid.length > 0 && (
        <div className="space-y-1 border border-destructive/40 bg-destructive/5 p-4 text-sm">
          <div className="flex items-center gap-2 font-medium text-destructive">
            <AlertCircle className="h-4 w-4" />
            {ui("Entry files that do not parse")}
          </div>
          {data.invalid.map((item) => (
            <div key={item.path} className="text-muted-foreground">
              <span className="font-mono text-foreground">{item.path}</span>: {item.error}
            </div>
          ))}
        </div>
      )}
      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={sections}
        errorMessagePrefix="Failed to load entries"
        skeletonCount={3}
        emptyState={
          <EmptyState
            icon={<MemoryIcon />}
            title={ui("Nothing remembered yet")}
            description={ui(
              "Agents record decisions, rules, facts, preferences and blocked items here with the remember tool.",
            )}
          />
        }
      >
        {(items) => (
          <>
            {items.map((section) => (
              <section key={section.key} className="space-y-3">
                <h2 className="text-sm font-medium text-muted-foreground">
                  {ui(section.title)} <span className="tabular-nums">{section.entries.length}</span>
                </h2>
                <div className="space-y-2">
                  {section.entries.map((entry) => (
                    <EntryRow key={entry.path} entry={entry} />
                  ))}
                </div>
              </section>
            ))}
          </>
        )}
      </QueryStateWrapper>
    </div>
  );
}

function EntryRow({ entry }: { entry: MemoryEntry }) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  return (
    <div className="space-y-2 border bg-card p-4">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Badge variant={entry.kind === "blocked" && !entry.resolved_at ? "accent" : "secondary"}>
          {ui(entry.kind)}
        </Badge>
        {entry.subject && <span className="font-mono text-foreground">{entry.subject}</span>}
        <span className="ml-auto font-mono text-muted-foreground">{entry.path}</span>
      </div>
      <p className="text-sm text-foreground">{entry.statement}</p>
      {entry.quote && (
        <blockquote className="border-l-2 pl-3 text-sm text-muted-foreground">
          {entry.quote}
        </blockquote>
      )}
      {entry.resolved_at && (
        <p className="text-xs text-muted-foreground">
          {ui("Resolved")} {formatRelativeTime(entry.resolved_at, displayLocale)}
          {entry.resolution ? `: ${entry.resolution}` : ""}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>{formatRelativeTime(entry.occurred_at, displayLocale)}</span>
        <span>
          {ui("by")} <span className="font-mono">{entry.recorded_by}</span>
        </span>
        {entry.source && (
          <span className="flex items-center gap-1">
            {entry.source.kind}
            {entry.source.reference && <span className="font-mono">{entry.source.reference}</span>}
            {entry.source.url && (
              <a
                href={entry.source.url}
                target="_blank"
                rel="noreferrer noopener"
                className="inline-flex items-center hover:text-foreground"
                aria-label={ui("Open source")}
              >
                <ExternalLink className="h-3 w-3" />
              </a>
            )}
          </span>
        )}
      </div>
    </div>
  );
}

function HistoryTab({ memoryId }: { memoryId: string }) {
  const ui = useUiTranslations();
  const { data, isLoading, error } = useMemoryHistory(memoryId);
  return (
    <QueryStateWrapper
      isLoading={isLoading}
      error={error}
      data={data}
      errorMessagePrefix="Failed to load history"
      skeletonCount={4}
      emptyState={<EmptyState icon={<MemoryIcon />} title={ui("No changes yet")} />}
    >
      {(versions) => (
        <div className="divide-y border bg-card">
          {versions.map((version, index) => (
            <VersionRow key={`${version.path}-${version.created_at}-${index}`} version={version} />
          ))}
        </div>
      )}
    </QueryStateWrapper>
  );
}

function VersionRow({ version }: { version: MemoryFileVersion }) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  const header = (
    <div className="flex flex-wrap items-center gap-2 text-sm">
      <Badge variant={version.action === "delete" ? "outline" : "secondary"}>
        {ui(version.action === "delete" ? "Deleted" : "Written")}
      </Badge>
      <span className="font-mono">{version.path}</span>
      <span className="ml-auto text-xs text-muted-foreground">
        {version.changed_by && <span className="mr-2 font-mono">{version.changed_by}</span>}
        {formatRelativeTime(version.created_at, displayLocale)}
      </span>
    </div>
  );
  if (!version.content) {
    return <div className="p-3">{header}</div>;
  }
  return (
    <details className="group p-3">
      <summary className="cursor-pointer list-none">{header}</summary>
      <pre className="mt-3 max-h-80 overflow-auto bg-muted/40 p-3 text-xs whitespace-pre-wrap">
        {version.content}
      </pre>
    </details>
  );
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
      <Skeleton className="h-24" />
      <Skeleton className="h-40" />
    </PageContainer>
  );
}
