"use client";

// Where a file browser reads from. The tree and viewer in this folder render any
// filesystem the same way: a session's workspace, a Knowledge collection, or
// anything else that can list a folder and read a file. Each surface supplies a
// `FileSource`; write operations are optional, and a source without them
// renders read-only.

import { createContext, useContext, useMemo } from "react";
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
import { throwApiError } from "@/lib/api/client";
import type { FileInfo, SessionFile } from "@/lib/api/types";

// Embedders whose API identity lives in a transport cannot navigate to cookie-
// authenticated URLs. Reads and downloads still use that transport.
export const FileNavigationContext = createContext({
  browserUrls: true,
  externalPreviewHref: undefined as string | undefined,
});

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
  download?(path: string): Promise<Blob>;
  externalPreviewHref?: string;
  /** Sandboxed HTML preview endpoint, when the source has one. */
  htmlPreviewUrl?(path: string): string | undefined;
  writes?: FileSourceWrites;
}

/** A session workspace (`/v1/workspaces/{id}/fs`), rooted at `/workspace`. */
export function workspaceFileSource(
  workspaceId: string | undefined,
  {
    readOnly = false,
    browserUrls = true,
    externalPreviewHref,
  }: { readOnly?: boolean; browserUrls?: boolean; externalPreviewHref?: string } = {},
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
    downloadUrl: (path) =>
      browserUrls && workspaceId ? fileDownloadUrl(workspaceId, path) : undefined,
    download: browserUrls
      ? undefined
      : async (path) => {
          const response = await fetch(fileDownloadUrl(id(), path));
          if (!response.ok) await throwApiError(response);
          return response.blob();
        },
    externalPreviewHref,
    htmlPreviewUrl: (path) =>
      browserUrls && workspaceId ? htmlPreviewUrl(workspaceId, path) : undefined,
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
  const navigation = useContext(FileNavigationContext);
  return useMemo(
    () => workspaceFileSource(workspaceId, { readOnly, ...navigation }),
    [workspaceId, readOnly, navigation],
  );
}
