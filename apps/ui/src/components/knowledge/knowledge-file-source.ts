"use client";

// A Knowledge collection as a `FileSource`, so the collection page browses it
// with the same tree and viewer as a session's filesystem. Folders come from the
// one-level listing (`tree=true`); files in the managed source accept writes,
// while files from connected sources and the built-in docs stay read-only.

import { useMemo } from "react";
import type { FileSource } from "@/components/files";
import { getPreviewType } from "@/components/files";
import { getFileExtension } from "@/lib/api/session-files";
import type { FileInfo, SessionFile } from "@/lib/api/types";
import { withOrgHeader } from "@/lib/api/active-org";
import {
  deleteKnowledgeFile,
  listKnowledgeFolder,
  writeKnowledgeFile,
  type KnowledgeCollection,
  type KnowledgeFile,
} from "@/lib/api/knowledge";

// Originals above this size are not pulled into the browser for a preview;
// the viewer offers the download instead.
const MAX_INLINE_ORIGINAL = 8 * 1024 * 1024;

export function knowledgeContentUrl(
  collectionId: string,
  path: string,
  { version, extracted = false }: { version?: string; extracted?: boolean } = {},
) {
  const params = new URLSearchParams({ path, extracted: String(extracted) });
  if (version) params.set("version", version);
  return `/api/v1/knowledge/${collectionId}/files/content?${params}`;
}

async function fetchContent(url: string): Promise<Response> {
  const response = await fetch(url, { headers: withOrgHeader(), credentials: "include" });
  if (!response.ok) throw new Error("File content unavailable");
  return response;
}

function toBase64(bytes: ArrayBuffer): string {
  let binary = "";
  const view = new Uint8Array(bytes);
  for (let i = 0; i < view.length; i += 0x8000) {
    binary += String.fromCharCode(...view.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

function entryInfo(
  collectionId: string,
  entry: { path: string; name: string; is_directory: boolean },
  readOnly: boolean,
  size = 0,
  id = entry.path,
): FileInfo {
  return {
    id,
    session_id: collectionId,
    path: entry.path,
    name: entry.name,
    filename: entry.name,
    content_type: "",
    metadata: {},
    is_directory: entry.is_directory,
    is_readonly: readOnly,
    size_bytes: size,
    created_at: "",
    updated_at: "",
  };
}

export { entryInfo as knowledgeFileInfo };

export interface KnowledgeFileSource extends FileSource {
  /** The catalog row behind a listed file, for its version, flags and extraction state. */
  fileAt(path: string): KnowledgeFile | undefined;
}

export function knowledgeFileSource(
  collection: KnowledgeCollection | undefined,
  canManage: boolean,
  /** Called after a folder lists, so a page can re-read `fileAt`. */
  onListed?: () => void,
): KnowledgeFileSource {
  const files = new Map<string, KnowledgeFile>();
  const id = collection?.id ?? "";
  const managed = collection?.sources?.find((source) => source.kind === "managed")?.id;
  const writable = canManage && !!managed;

  return {
    key: collection && `knowledge:${collection.id}`,
    root: "/",
    fileAt: (path) => files.get(path),
    async list(path) {
      const entries: FileInfo[] = [];
      let after = "";
      do {
        const page = await listKnowledgeFolder(id, path, after);
        for (const entry of page.data) {
          if (entry.kind === "folder") {
            entries.push(entryInfo(id, { ...entry, is_directory: true }, true));
          } else {
            files.set(entry.path, entry);
            entries.push(
              entryInfo(
                id,
                { ...entry, is_directory: false },
                !writable || entry.source_id !== managed,
                entry.version?.size_bytes ?? 0,
                entry.id,
              ),
            );
          }
        }
        after = page.next_cursor ?? "";
      } while (after);
      onListed?.();
      return entries;
    },
    async read(path): Promise<SessionFile> {
      const file = files.get(path);
      const info = entryInfo(
        id,
        { path, name: path.split("/").pop() ?? path, is_directory: false },
        true,
        file?.version?.size_bytes ?? 0,
        file?.id,
      );
      const version = file?.current_version;
      const size = file?.version?.size_bytes ?? 0;
      // Media and PDFs preview from their original bytes; everything else
      // shows the extracted text the agent reads.
      const media = getPreviewType(getFileExtension(path), "base64");
      if (media !== "binary" && size <= MAX_INLINE_ORIGINAL) {
        const response = await fetchContent(knowledgeContentUrl(id, path, { version }));
        return { ...info, content: toBase64(await response.arrayBuffer()), encoding: "base64" };
      }
      try {
        const response = await fetchContent(
          `${knowledgeContentUrl(id, path, { version, extracted: true })}&length=262144`,
        );
        return { ...info, content: await response.text(), encoding: "text" };
      } catch {
        // No text view (an unsupported binary): the viewer offers the download.
        return { ...info, content: undefined, encoding: "base64" };
      }
    },
    downloadUrl: (path) =>
      collection
        ? knowledgeContentUrl(id, path, { version: files.get(path)?.current_version })
        : undefined,
    writes: writable
      ? {
          createFile: (path, content) => writeKnowledgeFile(id, path, content),
          updateFile: (path, content) => writeKnowledgeFile(id, path, content),
          remove: (path) => deleteKnowledgeFile(id, path),
        }
      : undefined,
  };
}

export function useKnowledgeFileSource(
  collection: KnowledgeCollection | undefined,
  canManage: boolean,
  onListed?: () => void,
): KnowledgeFileSource {
  // Rebuild only when what the source depends on changes, not on every poll of
  // the collection (sync counts and jobs change while the tree stays put).
  const managed = collection?.sources?.find((source) => source.kind === "managed")?.id;
  const collectionId = collection?.id;
  return useMemo(
    () => knowledgeFileSource(collection, canManage, onListed),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [collectionId, managed, canManage],
  );
}
