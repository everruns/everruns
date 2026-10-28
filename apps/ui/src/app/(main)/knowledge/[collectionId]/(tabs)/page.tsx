"use client";
import { useConfirmation } from "@/components/ui/confirmation-provider";

// Files tab: the collection as a filesystem, in the same tree and viewer a
// session's Filesystem tab uses. The selected file is the `?file=` query param,
// so Search results and shared links open straight to it.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";
import { use, useReducer, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { File, FolderUp, Trash2, Upload } from "lucide-react";
import { FileBrowser, FileViewer } from "@/components/files";
import { PageMain } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { EntityStatus } from "@/components/ui/entity-status";
import { statusTone } from "@/components/knowledge/knowledge-status";
import {
  knowledgeFileInfo,
  useKnowledgeFileSource,
} from "@/components/knowledge/knowledge-file-source";
import {
  isBuiltinCollection,
  useCanManageKnowledge,
  useKnowledgeCollection,
  useRefreshKnowledge,
} from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";
import { formatRelativeTime } from "@/lib/formatting";
import {
  deleteKnowledgeFile,
  updateKnowledgeFileMetadata,
  uploadKnowledgeFile,
} from "@/lib/api/knowledge";
import type { FileInfo } from "@/lib/api/types";

const EXTRACTION_LABELS: Record<string, string> = {
  ready: "Text ready",
  failed: "Extraction failed",
  unsupported: "No text view",
  pending: "Extracting",
};

export default function KnowledgeFilesPage({
  params,
}: {
  params: Promise<{ collectionId: string }>;
}) {
  const { displayLocale } = useLocale();

  const requestConfirmation = useConfirmation();
  const ui = useUiTranslations();
  const { collectionId } = use(params);
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const canManage = useCanManageKnowledge();
  const collection = useKnowledgeCollection(collectionId);
  const c = collection.data;
  usePageTitle(c?.name, ui("Knowledge"));
  const refresh = useRefreshKnowledge();
  // A folder listing fills the source's file rows; re-render so the selected
  // file's flags and extraction state show once its folder has loaded.
  const [, listed] = useReducer((count: number) => count + 1, 0);
  const source = useKnowledgeFileSource(c, canManage, listed);
  const [refreshToken, setRefreshToken] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const uploadFiles = useRef<HTMLInputElement>(null);
  const uploadFolder = useRef<HTMLInputElement>(null);

  const selectedPath = searchParams.get("file") ?? undefined;
  useChatPageFilters({ file_path: selectedPath ?? "/" });
  const selectedRow = selectedPath ? source.fileAt(selectedPath) : undefined;
  const selected: FileInfo | null = selectedPath
    ? knowledgeFileInfo(
        collectionId,
        {
          path: selectedPath,
          name: selectedPath.split("/").pop() ?? selectedPath,
          is_directory: false,
        },
        !source.writes || selectedRow?.source_id !== managedSource(c),
        selectedRow?.version?.size_bytes ?? 0,
        selectedRow?.id,
      )
    : null;

  const select = (path: string | null) => {
    const next = new URLSearchParams(searchParams.toString());
    if (path) next.set("file", path);
    else next.delete("file");
    const query = next.toString();
    router.replace(query ? `${pathname}?${query}` : pathname, { scroll: false });
  };

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError("");
    try {
      await fn();
      await refresh();
      setRefreshToken((token) => token + 1);
    } catch (e) {
      setError(e instanceof Error ? e.message : "The action could not be completed.");
    } finally {
      setBusy(false);
    }
  };

  const upload = (list: FileList | null) => {
    const files = Array.from(list ?? []);
    if (!files.length) return;
    run(async () => {
      for (const file of files) {
        await uploadKnowledgeFile(collectionId, file, `/${file.webkitRelativePath || file.name}`);
      }
    });
  };

  const setFlag = (flag: "indexable" | "readable", value: boolean) =>
    selectedPath &&
    run(async () => {
      await updateKnowledgeFileMetadata(collectionId, { path: selectedPath, [flag]: value });
      // The listed row is the source of the checkbox; the tree reloads after
      // this, so mirror the change now instead of showing the old value.
      if (selectedRow) selectedRow[flag] = value;
    });

  const writable = !!source.writes && !isBuiltinCollection(c);
  const extraction = selectedRow?.version?.extraction_status;

  return (
    <PageMain className="space-y-3">
      {writable && (
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => uploadFiles.current?.click()}
          >
            <Upload className="size-4" />
            {ui("Upload files")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => uploadFolder.current?.click()}
          >
            <FolderUp className="size-4" />
            {ui("Upload folder")}
          </Button>
          <input
            ref={uploadFiles}
            type="file"
            multiple
            className="sr-only"
            aria-label={ui("Upload knowledge files")}
            onChange={(e) => {
              upload(e.target.files);
              e.target.value = "";
            }}
          />
          <input
            ref={(node) => {
              uploadFolder.current = node;
              node?.setAttribute("webkitdirectory", "");
            }}
            type="file"
            multiple
            className="sr-only"
            aria-label={ui("Upload knowledge folder")}
            onChange={(e) => {
              upload(e.target.files);
              e.target.value = "";
            }}
          />
        </div>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex h-[calc(100dvh-20rem)] min-h-[520px] flex-col overflow-hidden border bg-card @min-[48rem]/page-main:flex-row">
        <div className="h-64 shrink-0 overflow-hidden border-b bg-card/50 @min-[48rem]/page-main:h-auto @min-[48rem]/page-main:w-72 @min-[48rem]/page-main:border-r @min-[48rem]/page-main:border-b-0">
          <FileBrowser
            key={source.key}
            source={source}
            selectedPath={selectedPath}
            onFileSelect={(file) => select(file.path)}
            refreshToken={refreshToken}
            decorateFile={(file) => {
              const status = source.fileAt(file.path)?.version?.extraction_status;
              return status && status !== "ready" ? (
                <span
                  aria-label={ui(EXTRACTION_LABELS[status] ?? status)}
                  title={ui(EXTRACTION_LABELS[status] ?? status)}
                  className={
                    status === "failed"
                      ? "size-1.5 rounded-full bg-destructive"
                      : "size-1.5 rounded-full bg-muted-foreground/60"
                  }
                />
              ) : null;
            }}
            emptyState={
              <div className="flex flex-col items-center justify-center px-4 py-12 text-center text-muted-foreground">
                <File className="mb-2 size-8 opacity-40" />
                <p className="text-sm">{ui("No files yet")}</p>
                <p className="mt-1 text-xs">
                  {writable
                    ? ui("Upload files, write a note, or connect a source.")
                    : ui("Files appear here once a source syncs.")}
                </p>
              </div>
            }
          />
        </div>
        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {selected ? (
            <FileViewer
              key={selected.path}
              source={source}
              file={selected}
              onClose={() => select(null)}
              badges={
                extraction && (
                  <EntityStatus
                    status={extraction}
                    tone={statusTone(extraction)}
                    label={EXTRACTION_LABELS[extraction] ?? extraction}
                  />
                )
              }
              actions={
                source.writes &&
                !selected.is_readonly && (
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-destructive"
                    disabled={busy}
                    title={ui("Delete")}
                    onClick={async () => {
                      if (!(await requestConfirmation(`Delete ${selected.path}?`))) return;
                      run(async () => {
                        await deleteKnowledgeFile(collectionId, selected.path);
                        select(null);
                      });
                    }}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                )
              }
              footer={
                selectedRow && (
                  <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-t px-4 py-2.5 text-xs text-muted-foreground">
                    <span>{selectedRow.version?.mime_type}</span>
                    {selectedRow.version?.warnings?.length ? (
                      <span>
                        {ui("{value0} extraction warnings", {
                          value0: selectedRow.version.warnings.length,
                        })}
                      </span>
                    ) : null}
                    {selectedRow.version?.created_at && (
                      <span>
                        {ui("Updated {value0}", {
                          value0: formatRelativeTime(selectedRow.version.created_at, displayLocale),
                        })}
                      </span>
                    )}
                    {canManage && (
                      <span className="ml-auto flex flex-wrap items-center gap-4 text-foreground">
                        <label className="flex items-center gap-2">
                          <Checkbox
                            checked={selectedRow.readable}
                            disabled={busy}
                            onCheckedChange={(value) => setFlag("readable", !!value)}
                          />
                          {ui("Agent can read")}
                        </label>
                        {c?.indexing !== "off" && (
                          <label className="flex items-center gap-2">
                            <Checkbox
                              checked={selectedRow.indexable}
                              disabled={busy}
                              onCheckedChange={(value) => setFlag("indexable", !!value)}
                            />
                            {ui("Include in index")}
                          </label>
                        )}
                      </span>
                    )}
                  </div>
                )
              }
            />
          ) : (
            <div className="flex h-full flex-col items-center justify-center py-16 text-muted-foreground">
              <File className="mb-4 size-12 opacity-30" />
              <p className="text-sm font-medium">{ui("No file selected")}</p>
              <p className="mt-1 text-xs">{ui("Select a file to see what agents read")}</p>
            </div>
          )}
        </div>
      </div>
    </PageMain>
  );
}

function managedSource(collection: { sources?: { id: string; kind: string }[] } | undefined) {
  return collection?.sources?.find((source) => source.kind === "managed")?.id;
}
