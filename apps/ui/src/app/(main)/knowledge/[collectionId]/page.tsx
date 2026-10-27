"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";
import { use, useState, useRef } from "react";
import { useQuery, useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Download, RefreshCw, Search, Upload, FileText } from "lucide-react";
import { PageContainer, PageBreadcrumb, PageMasthead, PageMain } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { CollectionForm } from "@/components/knowledge/collection-form";
import { SourceForm } from "@/components/knowledge/source-form";
import { IndexerForm } from "@/components/knowledge/indexer-form";
import { useOrg } from "@/providers/org-provider";
import { usePageTitle } from "@/hooks/use-page-title";
import {
  updateKnowledgeFileMetadata,
  getKnowledge,
  listKnowledgeFiles,
  updateKnowledge,
  syncKnowledge,
  uploadKnowledgeFile,
  writeKnowledgeNote,
  searchKnowledge,
  knowledgeDownload,
  previewKnowledge,
  type KnowledgeFile,
  type KnowledgeResults,
} from "@/lib/api/knowledge";

export default function KnowledgeDetail({ params }: { params: Promise<{ collectionId: string }> }) {
  const ui = useUiTranslations();
  const { collectionId } = use(params);
  const { currentOrg } = useOrg();
  const cache = useQueryClient();
  const org = currentOrg?.public_id;
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  const collection = useQuery({
    queryKey: ["knowledge", org, collectionId],
    queryFn: () => getKnowledge(collectionId),
    enabled: !!org,
    refetchInterval: 5000,
  });
  const [prefix, setPrefix] = useState("/");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<KnowledgeResults | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<KnowledgeFile | null>(null);
  const [preview, setPreview] = useState("");
  const [notePath, setNotePath] = useState("notes/new-note.md");
  const [note, setNote] = useState("");
  useChatPageFilters({ search: query, file_path: selected?.path ?? prefix });
  const files = useInfiniteQuery({
    queryKey: ["knowledge-files", org, collectionId, prefix],
    queryFn: ({ pageParam }) => listKnowledgeFiles(collectionId, prefix, pageParam),
    initialPageParam: "",
    getNextPageParam: (p) => p.next_cursor ?? undefined,
    enabled: !!org,
    refetchInterval: 5000,
  });
  const refresh = async () => {
    await Promise.all([
      cache.invalidateQueries({ queryKey: ["knowledge"] }),
      cache.invalidateQueries({ queryKey: ["knowledge-files"] }),
      cache.invalidateQueries({ queryKey: ["knowledge-indexers"] }),
    ]);
  };
  usePageTitle(collection.data?.name, ui("Knowledge"));
  const action = async (fn: () => Promise<unknown>) => {
    setError("");
    setBusy(true);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "The action could not be completed.");
    } finally {
      setBusy(false);
    }
  };
  const previewRequest = useRef(0);
  const openFile = async (file: KnowledgeFile) => {
    const request = ++previewRequest.current;
    setSelected(file);
    setPreview("Loading text view…");
    try {
      const text = await previewKnowledge(collectionId, file);
      if (request === previewRequest.current) setPreview(text);
    } catch (e) {
      if (request === previewRequest.current)
        setPreview(e instanceof Error ? e.message : "Preview unavailable");
    }
  };
  const c = collection.data;
  const builtin = c?.name === "everruns-docs";
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Knowledge"), href: "/knowledge" }, { label: c?.name ?? "Collection" }]}
      />
      <PageMasthead
        icon={<BookOpen />}
        title={c?.name ?? ui("Knowledge")}
        description={c?.description ?? ui("Loading collection…")}
        actions={
          canManage && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => action(() => syncKnowledge(collectionId))}
            >
              <RefreshCw className="size-4" />
              {ui("Refresh sources")}
            </Button>
          )
        }
      />
      <PageMain>
        {(error || collection.error) && (
          <p role="alert" className="mb-4 text-sm text-destructive">
            {error || ui("Collection unavailable in this organization.")}
          </p>
        )}
        {c && (
          <>
            <div className="mb-6 flex flex-wrap items-center gap-3 text-sm">
              <Badge variant="secondary">
                {c.indexing === "off"
                  ? ui("Indexing off")
                  : c.indexing === "lexical"
                    ? ui("Keyword search")
                    : ui("Hybrid search")}
              </Badge>
              <span className="text-muted-foreground">
                {c.file_count ?? 0} {ui("files ·")} {c.indexed_count ?? 0} {ui("indexed")}
              </span>
              {c.revision !== c.published_revision && c.indexing !== "off" && (
                <Badge variant="outline">{ui("Building updated index")}</Badge>
              )}
              {c.jobs?.some((j) => j.status === "failed") && (
                <Badge variant="destructive">{ui("Processing needs attention")}</Badge>
              )}
            </div>
            {c.jobs
              ?.filter((j) => j.error)
              .slice(0, 1)
              .map((j) => (
                <p key={j.id} className="mb-4 text-sm text-muted-foreground">
                  {j.error} {ui("Refresh sources to retry.")}
                </p>
              ))}
            <section
              aria-label={ui("Search knowledge")}
              className="mb-6 rounded-lg border bg-card p-4"
            >
              <form
                className="flex gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  action(async () => setResults(await searchKnowledge(collectionId, query)));
                }}
              >
                <Input
                  aria-label={ui("Search collection")}
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={
                    c.indexing === "off"
                      ? ui("Enable indexing in settings to search passages")
                      : ui("Find relevant passages…")
                  }
                  disabled={c.indexing === "off"}
                />
                <Button type="submit" disabled={busy || !query.trim() || c.indexing === "off"}>
                  <Search className="size-4" />
                  {ui("Search")}
                </Button>
              </form>
              {c.indexing === "off" && (
                <p className="mt-2 text-xs text-muted-foreground">
                  {ui("Browse files below, or use grep on the agent's Knowledge mount.")}
                </p>
              )}
              {results && (
                <div className="mt-4 space-y-3">
                  {results.warnings.map((w) => (
                    <p key={w} className="text-xs text-muted-foreground">
                      {w}
                    </p>
                  ))}
                  {results.results.map((hit) => (
                    <article key={hit.id} className="rounded-md border p-3">
                      <h3 className="text-sm font-medium">{hit.title}</h3>
                      <p className="my-2 whitespace-pre-wrap text-sm text-muted-foreground">
                        {hit.snippet}
                      </p>
                      <p className="break-all font-mono text-xs text-muted-foreground">
                        {hit.path}
                      </p>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {ui("Version")} {hit.version_id}
                      </p>
                      {/^https:\/\//.test(hit.source_uri) && (
                        <a
                          className="text-xs underline"
                          href={hit.source_uri}
                          target="_blank"
                          rel="noreferrer"
                        >
                          {ui("Original source")}
                        </a>
                      )}
                    </article>
                  ))}
                  {!results.results.length && (
                    <p className="text-sm text-muted-foreground">
                      {ui("No matching indexed passages.")}
                    </p>
                  )}
                  {results.truncated && (
                    <p className="text-xs text-muted-foreground">
                      {ui("Results are bounded. Narrow the query for more complete coverage.")}
                    </p>
                  )}
                </div>
              )}
            </section>
            <div className="grid items-start gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
              <section className="min-w-0 rounded-lg border">
                <div className="flex flex-wrap items-center gap-3 border-b p-4">
                  <h2 className="font-medium">{ui("Files")}</h2>
                  <Input
                    aria-label={ui("Filter file paths")}
                    className="ml-auto max-w-xs"
                    value={prefix}
                    onChange={(e) => setPrefix(e.target.value)}
                    placeholder="/docs/"
                  />
                  {canManage && !builtin && (
                    <label className="inline-flex cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-xs">
                      <Upload className="size-4" />
                      {ui("Upload")}
                      <input
                        type="file"
                        multiple
                        className="sr-only"
                        aria-label={ui("Upload knowledge files")}
                        disabled={busy}
                        onChange={(e) => {
                          const uploads = Array.from(e.target.files ?? []);
                          action(async () => {
                            for (const file of uploads) {
                              await uploadKnowledgeFile(
                                collectionId,
                                file,
                                `${prefix.replace(/\/$/, "")}/${file.webkitRelativePath || file.name}`,
                              );
                            }
                          });
                          e.target.value = "";
                        }}
                      />
                    </label>
                  )}
                  {canManage && !builtin && (
                    <label className="inline-flex cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-xs">
                      <Upload className="size-4" />
                      {ui("Upload folder")}
                      <input
                        type="file"
                        multiple
                        className="sr-only"
                        aria-label={ui("Upload knowledge folder")}
                        ref={(node) => {
                          node?.setAttribute("webkitdirectory", "");
                        }}
                        disabled={busy}
                        onChange={(e) => {
                          const uploads = Array.from(e.target.files ?? []);
                          action(async () => {
                            for (const file of uploads) {
                              await uploadKnowledgeFile(
                                collectionId,
                                file,
                                `${prefix.replace(/\/$/, "")}/${file.webkitRelativePath || file.name}`,
                              );
                            }
                          });
                          e.target.value = "";
                        }}
                      />
                    </label>
                  )}
                </div>
                {files.error && (
                  <p role="alert" className="p-4 text-sm text-destructive">
                    {ui("Unable to load files.")}
                  </p>
                )}
                <div className="max-h-[560px] overflow-auto divide-y">
                  {files.data?.pages
                    .flatMap((p) => p.data)
                    .map((file) => (
                      <button
                        key={file.id}
                        className={`flex w-full items-start gap-3 p-3 text-left hover:bg-muted/50 ${selected?.id === file.id ? "bg-muted" : ""}`}
                        onClick={() => openFile(file)}
                      >
                        <FileText className="mt-1 size-4 shrink-0 text-muted-foreground" />
                        <span className="min-w-0 flex-1">
                          <span className="block break-all text-sm">{file.path}</span>
                          <span className="mt-1 block text-xs text-muted-foreground">
                            {file.version?.mime_type ?? ui("Original file")} ·{" "}
                            {file.version?.size_bytes !== undefined
                              ? ui("{value0} bytes", {
                                  value0: file.version.size_bytes.toLocaleString(),
                                })
                              : file.indexed_version
                                ? ui("Indexed")
                                : ui("Not indexed")}
                          </span>
                        </span>
                        {file.version?.extraction_status && (
                          <Badge variant="outline">{file.version.extraction_status}</Badge>
                        )}
                      </button>
                    ))}
                </div>
                {!files.isLoading && !files.data?.pages.some((p) => p.data.length) && (
                  <p className="p-5 text-sm text-muted-foreground">
                    {ui("No files in this view. Upload files or connect a source below.")}
                  </p>
                )}
                {files.hasNextPage && (
                  <Button
                    className="m-3"
                    variant="outline"
                    onClick={() => files.fetchNextPage()}
                    disabled={files.isFetchingNextPage}
                  >
                    {ui("Load more files")}
                  </Button>
                )}
              </section>
              <section className="min-w-0 rounded-lg border bg-card p-4">
                <div className="mb-3 flex items-start gap-2">
                  <h2 className="min-w-0 flex-1 break-all text-sm font-medium">
                    {selected?.path ?? ui("File preview")}
                  </h2>
                  {selected && (
                    <a
                      href={knowledgeDownload(collectionId, selected)}
                      download
                      className="flex items-center gap-1 text-xs underline"
                    >
                      <Download className="size-3" />
                      {ui("Original")}
                    </a>
                  )}
                </div>
                <pre className="max-h-[480px] overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/40 p-3 font-mono text-xs">
                  {preview ||
                    ui(
                      "Select a file to inspect its extracted text. Binary originals are retained even when no text view is available.",
                    )}
                </pre>
                {selected && canManage && (
                  <div className="mt-4 flex flex-wrap gap-4 text-sm">
                    <label className="flex items-center gap-2">
                      <Checkbox
                        checked={selected.indexable}
                        disabled={busy}
                        onCheckedChange={(value) =>
                          action(async () => {
                            const result = await updateKnowledgeFileMetadata(collectionId, {
                              path: selected.path,
                              indexable: !!value,
                            });
                            setSelected({
                              ...selected,
                              current_version: result.version_id,
                              indexable: !!value,
                            });
                          })
                        }
                      />
                      {ui("Include in index")}
                    </label>
                    <label className="flex items-center gap-2">
                      <Checkbox
                        checked={selected.readable}
                        disabled={busy}
                        onCheckedChange={(value) =>
                          action(async () => {
                            const result = await updateKnowledgeFileMetadata(collectionId, {
                              path: selected.path,
                              readable: !!value,
                            });
                            setSelected({
                              ...selected,
                              current_version: result.version_id,
                              readable: !!value,
                            });
                          })
                        }
                      />
                      {ui("Agent can read")}
                    </label>
                  </div>
                )}
                {selected?.version && (
                  <details className="mt-4">
                    <summary className="cursor-pointer text-sm">
                      {ui("Metadata and extraction warnings")}
                    </summary>
                    <pre className="mt-3 overflow-auto whitespace-pre-wrap text-xs">
                      {JSON.stringify(
                        {
                          metadata: selected.version.metadata,
                          warnings: selected.version.warnings,
                        },
                        null,
                        2,
                      )}
                    </pre>
                  </details>
                )}
              </section>
            </div>
            <section className="mt-8 space-y-4">
              <h2 className="text-lg font-medium">{ui("Sources")}</h2>
              {c.sources?.map((source) => (
                <div key={source.id} className="rounded-lg border p-4">
                  <div className="flex gap-3">
                    <span className="font-medium capitalize">
                      {source.kind === "builtin" ? ui("Everruns documentation") : source.kind}
                    </span>
                    <Badge variant="outline">{source.sync_status}</Badge>
                  </div>
                  <p className="mt-2 break-all text-sm text-muted-foreground">
                    {source.config.repository ??
                      source.config.url ??
                      (source.kind === "managed"
                        ? "Your notes and uploaded files"
                        : "Versioned public documentation snapshot")}
                    {source.root_path && ` → /${source.root_path}`}
                  </p>
                  {source.last_error && (
                    <p role="alert" className="mt-2 text-sm text-destructive">
                      {source.last_error}
                    </p>
                  )}
                </div>
              ))}
            </section>
            {canManage && !builtin && (
              <details className="mt-5 rounded-lg border p-5">
                <summary className="cursor-pointer font-medium">{ui("Connect a source")}</summary>
                <div className="mt-5 max-w-2xl">
                  <SourceForm collectionId={collectionId} onSaved={refresh} />
                </div>
              </details>
            )}
            {canManage && !builtin && (
              <details className="mt-5 rounded-lg border p-5">
                <summary className="cursor-pointer font-medium">
                  {ui("Write a note or OKF concept")}
                </summary>
                <form
                  className="mt-5 max-w-2xl space-y-3"
                  onSubmit={(e) => {
                    e.preventDefault();
                    action(async () => {
                      await writeKnowledgeNote(collectionId, notePath, note);
                      setNote("");
                    });
                  }}
                >
                  <Label htmlFor="knowledge-note-path">{ui("File path")}</Label>
                  <Input
                    id="knowledge-note-path"
                    value={notePath}
                    onChange={(e) => setNotePath(e.target.value)}
                    required
                  />
                  <Label htmlFor="knowledge-note">{ui("Markdown and frontmatter")}</Label>
                  <Textarea
                    id="knowledge-note"
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                    rows={9}
                    placeholder={ui("Write Markdown, optionally with OKF metadata…")}
                  />
                  <Button type="submit" disabled={busy || !note.trim()}>
                    {ui("Save note")}
                  </Button>
                </form>
              </details>
            )}
            {canManage && (
              <details className="mt-5 rounded-lg border p-5">
                <summary className="cursor-pointer font-medium">
                  {ui("Collection settings")}
                </summary>
                <div className="mt-5 max-w-2xl">
                  <CollectionForm
                    key={c.id}
                    initial={c}
                    onSave={async (input) => {
                      await updateKnowledge(collectionId, input);
                      await refresh();
                    }}
                  />
                </div>
              </details>
            )}
            {canManage && (
              <details className="mt-5 rounded-lg border p-5">
                <summary className="cursor-pointer font-medium">
                  {ui("Add an organization search account")}
                </summary>
                <div className="mt-5 max-w-2xl">
                  <IndexerForm onSaved={refresh} />
                </div>
              </details>
            )}
          </>
        )}
      </PageMain>
    </PageContainer>
  );
}
