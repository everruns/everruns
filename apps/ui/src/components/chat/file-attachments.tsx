"use client";

import { FileText, Loader2, X, AlertCircle } from "lucide-react";
import type { PendingFile } from "@/lib/api/files";

interface FileAttachmentsProps {
  files: PendingFile[];
  onRemove: (id: string) => void;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function FileAttachments({ files, onRemove }: FileAttachmentsProps) {
  if (files.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-2 px-4 pt-2">
      {files.map((f) => (
        <div
          key={f.id}
          className="flex items-center gap-2 rounded-lg border border-border bg-muted/50 px-2.5 py-1.5 text-sm"
        >
          {f.status === "uploading" ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          ) : f.status === "error" ? (
            <AlertCircle className="h-4 w-4 text-destructive" />
          ) : (
            <FileText className="h-4 w-4 text-muted-foreground" />
          )}
          <div className="flex min-w-0 flex-col">
            <span className="max-w-40 truncate font-medium">{f.file.name}</span>
            <span className="text-xs text-muted-foreground">
              {f.status === "error" ? (f.error ?? "Upload failed") : formatSize(f.file.size)}
            </span>
          </div>
          <button
            type="button"
            onClick={() => onRemove(f.id)}
            className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label={`Remove ${f.file.name}`}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
