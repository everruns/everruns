"use client";

// Sources tab: where a collection's files come from and where each source is
// mounted in its tree. "Refresh sources" in the page header syncs them all.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { use, useState } from "react";
import { Plus } from "lucide-react";
import { PageMain } from "@/components/layout";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { SourceForm } from "@/components/knowledge/source-form";
import { EntityStatus } from "@/components/ui/entity-status";
import { statusLabel, statusTone } from "@/components/knowledge/knowledge-status";
import {
  isBuiltinCollection,
  useCanManageKnowledge,
  useKnowledgeCollection,
  useRefreshKnowledge,
} from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";
import { useKnowledgeSourceRows } from "@/components/knowledge/use-knowledge-page-context";
import { formatRelativeTime } from "@/lib/formatting";
import type { KnowledgeSource } from "@/lib/api/knowledge";

function sourceName(source: KnowledgeSource) {
  if (source.kind === "managed") return "Managed files";
  if (source.kind === "builtin") return "Everruns documentation";
  const where = source.config.repository ?? source.config.url ?? "";
  return `${source.kind === "github" ? "GitHub" : "Git"} ${where}`.trim();
}

function sourceDetail(source: KnowledgeSource) {
  if (source.kind === "managed") return "Uploads and notes written here";
  if (source.kind === "builtin") return "Versioned public documentation snapshot";
  return [source.config.branch, source.config.root_folder && `/${source.config.root_folder}`]
    .filter(Boolean)
    .join(" · ");
}

export default function KnowledgeSourcesPage({
  params,
}: {
  params: Promise<{ collectionId: string }>;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const { collectionId } = use(params);
  const collection = useKnowledgeCollection(collectionId);
  const canManage = useCanManageKnowledge();
  const refresh = useRefreshKnowledge();
  const [connecting, setConnecting] = useState(false);
  const c = collection.data;
  useKnowledgeSourceRows(c?.sources ?? [], {
    loading: collection.isLoading,
    error: !!collection.error,
  });
  usePageTitle(ui("Sources"), c?.name ?? ui("Knowledge"));
  const canConnect = canManage && !!c && !isBuiltinCollection(c);

  return (
    <PageMain className="space-y-4">
      {canConnect && (
        <div className="flex justify-end">
          <Button variant="outline" size="sm" onClick={() => setConnecting(true)}>
            <Plus className="size-4" />
            {ui("Connect a source")}
          </Button>
        </div>
      )}
      <div className="border bg-card">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{ui("Source")}</TableHead>
              <TableHead>{ui("Mounted at")}</TableHead>
              <TableHead>{ui("Status")}</TableHead>
              <TableHead>{ui("Last sync")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {(c?.sources ?? []).map((source) => (
              <TableRow key={source.id}>
                <TableCell className="align-top">
                  <div className="font-medium">{ui(sourceName(source))}</div>
                  <div className="text-xs text-muted-foreground">{ui(sourceDetail(source))}</div>
                  {source.last_error && (
                    <p role="alert" className="mt-1 text-xs text-destructive">
                      {source.last_error}
                    </p>
                  )}
                </TableCell>
                <TableCell className="align-top font-mono text-xs">
                  /{source.root_path.replace(/^\/+/, "")}
                </TableCell>
                <TableCell className="align-top">
                  <EntityStatus
                    status={source.sync_status}
                    tone={source.last_error ? "destructive" : statusTone(source.sync_status)}
                    label={statusLabel(source.sync_status)}
                  />
                </TableCell>
                <TableCell className="align-top text-muted-foreground">
                  {source.last_synced_at
                    ? formatRelativeTime(source.last_synced_at, displayLocale)
                    : ui("Never")}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>

      <Dialog open={connecting} onOpenChange={setConnecting}>
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>{ui("Connect a source")}</DialogTitle>
            <DialogDescription>
              {ui("Files from the repository are mounted read-only under the folder you choose.")}
            </DialogDescription>
          </DialogHeader>
          <SourceForm
            collectionId={collectionId}
            onSaved={async () => {
              setConnecting(false);
              await refresh();
            }}
          />
        </DialogContent>
      </Dialog>
    </PageMain>
  );
}
