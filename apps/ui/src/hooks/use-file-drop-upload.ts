// Hook for drag-and-drop file upload to session filesystem.
// Reads dropped files from the OS, determines text vs binary encoding,
// and uploads them via the session files API.
"use client";

import { useState, useCallback, useRef } from "react";
import { createFile, joinPath } from "@/lib/api/session-files";

export interface FileUploadItem {
  name: string;
  status: "reading" | "uploading" | "done" | "error";
  error?: string;
}

const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "mdx",
  "json",
  "yaml",
  "yml",
  "toml",
  "xml",
  "html",
  "htm",
  "css",
  "scss",
  "less",
  "sass",
  "js",
  "jsx",
  "ts",
  "tsx",
  "mjs",
  "cjs",
  "py",
  "rs",
  "go",
  "java",
  "c",
  "cpp",
  "h",
  "hpp",
  "rb",
  "php",
  "swift",
  "kt",
  "scala",
  "sh",
  "bash",
  "zsh",
  "fish",
  "sql",
  "graphql",
  "gql",
  "vue",
  "svelte",
  "astro",
  "csv",
  "tsv",
  "log",
  "env",
  "ini",
  "cfg",
  "conf",
  "gitignore",
  "gitattributes",
  "editorconfig",
]);

const TEXT_MIME_TYPES = new Set([
  "application/json",
  "application/xml",
  "application/javascript",
  "application/typescript",
  "application/x-yaml",
  "application/x-toml",
  "application/x-sh",
  "application/xhtml+xml",
  "application/sql",
]);

function isTextFile(file: File): boolean {
  if (file.type.startsWith("text/")) return true;
  if (TEXT_MIME_TYPES.has(file.type)) return true;
  // No MIME type — check extension
  if (!file.type) {
    const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
    return TEXT_EXTENSIONS.has(ext);
  }
  return false;
}

function readAsText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(new Error("Failed to read file"));
    reader.readAsText(file);
  });
}

function readAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const arrayBuffer = reader.result as ArrayBuffer;
      const bytes = new Uint8Array(arrayBuffer);
      let binary = "";
      for (let i = 0; i < bytes.length; i++) {
        binary += String.fromCharCode(bytes[i]);
      }
      resolve(btoa(binary));
    };
    reader.onerror = () => reject(new Error("Failed to read file"));
    reader.readAsArrayBuffer(file);
  });
}

export function useFileDropUpload(
  sessionId: string,
  targetDir: string = "/workspace",
  onComplete?: () => void,
) {
  const [isDragging, setIsDragging] = useState(false);
  const [uploads, setUploads] = useState<FileUploadItem[]>([]);
  const dragCounter = useRef(0);

  const uploadFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;

      const items: FileUploadItem[] = files.map((f) => ({
        name: f.name,
        status: "reading" as const,
      }));
      setUploads(items);

      await Promise.all(
        files.map(async (file, i) => {
          try {
            const text = isTextFile(file);
            const content = text ? await readAsText(file) : await readAsBase64(file);
            const encoding = text ? "text" : "base64";

            setUploads((prev) =>
              prev.map((item, j) => (j === i ? { ...item, status: "uploading" as const } : item)),
            );

            await createFile(sessionId, {
              path: joinPath(targetDir, file.name),
              content,
              encoding,
            });

            setUploads((prev) =>
              prev.map((item, j) => (j === i ? { ...item, status: "done" as const } : item)),
            );
          } catch (err) {
            setUploads((prev) =>
              prev.map((item, j) =>
                j === i
                  ? {
                      ...item,
                      status: "error" as const,
                      error: err instanceof Error ? err.message : "Upload failed",
                    }
                  : item,
              ),
            );
          }
        }),
      );

      onComplete?.();
      setTimeout(() => setUploads([]), 3000);
    },
    [sessionId, targetDir, onComplete],
  );

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current++;
    if (dragCounter.current === 1) {
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current--;
    if (dragCounter.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      dragCounter.current = 0;
      setIsDragging(false);

      const droppedFiles = e.dataTransfer?.files;
      if (!droppedFiles || droppedFiles.length === 0) return;

      uploadFiles(Array.from(droppedFiles));
    },
    [uploadFiles],
  );

  return {
    isDragging,
    uploads,
    isUploading: uploads.some((u) => u.status === "reading" || u.status === "uploading"),
    dragHandlers: {
      onDragEnter: handleDragEnter,
      onDragLeave: handleDragLeave,
      onDragOver: handleDragOver,
      onDrop: handleDrop,
    },
  };
}
