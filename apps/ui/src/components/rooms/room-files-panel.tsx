"use client";

// A room's shared folder: the files as they are now, one file's content at any
// version, and that file's history with who made each change.

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileText } from "lucide-react";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { Skeleton } from "@/components/ui/skeleton";
import {
  listRoomFileHistory,
  listRoomFiles,
  readRoomFile,
  type RoomFileVersion,
} from "@/lib/api/rooms";
import { formatDate, formatFileSize } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

export function RoomFilesPanel({
  roomId,
  refetchInterval,
}: {
  roomId: string;
  refetchInterval?: number | false;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const [selected, setSelected] = useState<string | null>(null);
  const [version, setVersion] = useState<number | undefined>(undefined);
  const { data: files, isLoading } = useQuery({
    queryKey: ["rooms", roomId, "files"],
    queryFn: () => listRoomFiles(roomId),
    refetchInterval,
  });
  const path = selected ?? files?.[0]?.path ?? null;

  if (isLoading) return <Skeleton className="h-48 w-full" />;
  if (!files?.length) {
    return (
      <div className="flex flex-col items-center gap-2 border bg-card px-6 py-10 text-center">
        <FileText className="size-5 text-muted-foreground" />
        <div className="text-sm font-semibold">{ui("No files yet")}</div>
        <p className="max-w-md text-[13px] text-muted-foreground">
          {ui(
            "Members keep notes, plans and decisions here. Agents in the room see the folder at /room and every change is kept.",
          )}
        </p>
      </div>
    );
  }

  return (
    <div className="grid gap-4 lg:grid-cols-[260px_minmax(0,1fr)]">
      <ul className="border bg-card" aria-label={ui("Files")}>
        {files.map((file) => (
          <li key={file.path}>
            <button
              type="button"
              aria-current={file.path === path ? "true" : undefined}
              onClick={() => {
                setSelected(file.path);
                setVersion(undefined);
              }}
              className={cn(
                "flex w-full flex-col items-start gap-0.5 border-b px-3 py-2 text-left last:border-b-0 hover:bg-muted",
                file.path === path && "bg-muted",
              )}
            >
              <span className="w-full truncate font-mono text-[13px]">{file.path}</span>
              <span className="text-[11px] text-muted-foreground">
                v{file.version} · {file.updated_by?.display_name ?? ui("unknown")} ·{" "}
                {formatDate(file.updated_at, displayLocale)}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {path && (
        <RoomFileView
          key={path}
          roomId={roomId}
          path={path}
          version={version}
          onVersion={setVersion}
          refetchInterval={refetchInterval}
        />
      )}
    </div>
  );
}

function RoomFileView({
  roomId,
  path,
  version,
  onVersion,
  refetchInterval,
}: {
  roomId: string;
  path: string;
  version?: number;
  onVersion: (version: number | undefined) => void;
  refetchInterval?: number | false;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const { data: content, isLoading } = useQuery({
    queryKey: ["rooms", roomId, "file", path, version ?? "current"],
    queryFn: () => readRoomFile(roomId, path, version),
    refetchInterval: version === undefined ? refetchInterval : false,
  });
  const { data: history } = useQuery({
    queryKey: ["rooms", roomId, "file-history", path],
    queryFn: () => listRoomFileHistory(roomId, path),
    refetchInterval,
  });
  const actionLabel = (entry: RoomFileVersion) => {
    switch (entry.action) {
      case "write":
        return ui("wrote");
      case "move":
        return `${ui("moved from")} ${entry.moved_from ?? ""}`.trim();
      case "delete":
        return ui("deleted");
      case "restore":
        return ui("restored");
      default:
        return entry.action;
    }
  };
  const latest = history?.[0]?.version;

  return (
    <div className="grid min-w-0 gap-4 xl:grid-cols-[minmax(0,1fr)_240px]">
      <section className="min-w-0 border bg-card" aria-label={path}>
        <header className="flex flex-wrap items-center justify-between gap-2 border-b px-4 py-2.5">
          <span className="font-mono text-[13px] font-medium">{path}</span>
          {content && (
            <span className="text-[11px] text-muted-foreground">
              {ui("Version")} {content.version}
              {version !== undefined && latest !== undefined && version !== latest
                ? ` · ${ui("not the current version")}`
                : ""}{" "}
              · {formatFileSize(content.size_bytes)}
            </span>
          )}
        </header>
        <div className="p-4">
          {isLoading || !content ? (
            <Skeleton className="h-32 w-full" />
          ) : content.encoding !== "text" ? (
            <p className="text-sm text-muted-foreground">
              {ui("This file is not text, so it is not shown here.")}
            </p>
          ) : path.endsWith(".md") ? (
            <MarkdownDisplay content={content.content} />
          ) : (
            <pre className="overflow-x-auto font-mono text-[12px] whitespace-pre-wrap">
              {content.content}
            </pre>
          )}
        </div>
      </section>
      <section className="border bg-card" aria-label={ui("History")}>
        <h3 className="border-b px-3 py-2.5 text-xs font-medium text-muted-foreground">
          {ui("History")}
        </h3>
        <ol>
          {(history ?? []).map((entry) => {
            const current = (version ?? latest) === entry.version;
            return (
              <li key={entry.version}>
                <button
                  type="button"
                  disabled={entry.action === "delete"}
                  onClick={() => onVersion(entry.version === latest ? undefined : entry.version)}
                  className={cn(
                    "flex w-full flex-col items-start gap-0.5 border-b px-3 py-2 text-left text-[12px] last:border-b-0 hover:bg-muted disabled:cursor-default disabled:hover:bg-transparent",
                    current && "bg-muted",
                  )}
                >
                  <span>
                    <span className="font-medium">v{entry.version}</span>{" "}
                    {entry.author?.display_name ?? ui("unknown")} {actionLabel(entry)}
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    {formatDate(entry.created_at, displayLocale)}
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      </section>
    </div>
  );
}
