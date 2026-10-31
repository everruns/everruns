"use client";

// Where a file browser reads from. The tree and viewer in this folder render any
// filesystem the same way: a session's workspace, a Knowledge collection, or
// anything else that can list a folder and read a file. Each surface supplies a
// `FileSource`; write operations are optional, and a source without them
// renders read-only.

import { useMemo } from "react";
import {
  createFile,
  deleteFile,
  fileDownloadUrl,
  htmlPreviewUrl,
  listFiles,
  mkdir,
  readFile,
  updateFile,
} from "@/lib/api/session-files";
import type { FileInfo, SessionFile } from "@/lib/api/types";

export interface FileSourceWrites {
  createFile(path: string, content: string): Promise<unknown>;
  updateFile(path: string, content: string): Promise<unknown>;
  /** Absent when folders exist only through the files in them. */
  mkdir?(path: string): Promise<unknown>;
  remove(path: string, recursive: boolean): Promise<unknown>;
}

export interface FileSource {
  /** Cache key for this filesystem; undefined while its owner is still loading. */
  key: string | undefined;
  /** The folder the tree opens at. */
  root: string;
  list(path: string): Promise<FileInfo[]>;
  read(path: string): Promise<SessionFile>;
  /** Raw bytes with their content type, for download and media playback. */
  downloadUrl(path: string): string | undefined;
  /** Sandboxed HTML preview endpoint, when the source has one. */
  htmlPreviewUrl?(path: string): string | undefined;
  writes?: FileSourceWrites;
}

/** A session workspace (`/v1/workspaces/{id}/fs`), rooted at `/workspace`. */
export function workspaceFileSource(
  workspaceId: string | undefined,
  { readOnly = false }: { readOnly?: boolean } = {},
): FileSource {
  const id = () => {
    if (!workspaceId) throw new Error("Workspace is still loading");
    return workspaceId;
  };
  return {
    key: workspaceId && `workspace:${workspaceId}`,
    root: "/workspace",
    list: (path) => listFiles(id(), path),
    read: (path) => readFile(id(), path),
    downloadUrl: (path) => workspaceId && fileDownloadUrl(workspaceId, path),
    htmlPreviewUrl: (path) => workspaceId && htmlPreviewUrl(workspaceId, path),
    writes: readOnly
      ? undefined
      : {
          createFile: (path, content) => createFile(id(), { path, content, encoding: "text" }),
          updateFile: (path, content) => updateFile(id(), path, { content, encoding: "text" }),
          mkdir: (path) => mkdir(id(), path),
          remove: (path, recursive) => deleteFile(id(), path, recursive),
        },
  };
}

export function useWorkspaceFileSource(
  workspaceId: string | undefined,
  options: { readOnly?: boolean } = {},
): FileSource {
  const { readOnly = false } = options;
  return useMemo(() => workspaceFileSource(workspaceId, { readOnly }), [workspaceId, readOnly]);
}
