"use client";

import { useState, useCallback, useEffect } from "react";
import type { PendingFile } from "@/lib/api/files";
import {
  createPendingFile,
  cleanupPendingFiles,
  validateFileUpload,
  uploadFile,
} from "@/lib/api/files";

interface UseFileAttachmentsOptions {
  sessionId?: string;
  maxFiles?: number;
}

/**
 * Hook to manage file (PDF) attachments for chat input.
 *
 * Mirrors useImageAttachments:
 * - Adding files (from picker, drop, or paste)
 * - Uploading files in parallel
 * - Tracking upload status
 * - Removing files
 */
export function useFileAttachments(options: UseFileAttachmentsOptions = {}) {
  const { sessionId, maxFiles = 5 } = options;

  const [pendingFiles, setPendingFiles] = useState<PendingFile[]>([]);

  const allUploaded = pendingFiles.every((f) => f.status === "ready" || f.status === "error");

  const uploadedFileIds = pendingFiles
    .filter((f) => f.status === "ready" && f.uploadedFileId)
    .map((f) => ({
      fileId: f.uploadedFileId!,
      filename: f.uploadedFilename ?? f.file.name,
    }));

  useEffect(() => {
    return () => {
      cleanupPendingFiles(pendingFiles);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const addFiles = useCallback(
    async (files: File[]) => {
      const validFiles: File[] = [];
      const errors: string[] = [];

      for (const file of files) {
        const error = validateFileUpload(file);
        if (!error) {
          validFiles.push(file);
        } else {
          errors.push(error);
        }
      }

      const remainingSlots = maxFiles - pendingFiles.length;
      const filesToAdd = validFiles.slice(0, remainingSlots);

      if (filesToAdd.length < validFiles.length) {
        errors.push(`Only ${maxFiles} files allowed. Some files were not added.`);
      }

      if (errors.length > 0) {
        console.warn("File validation errors:", errors);
      }

      if (filesToAdd.length === 0) return;

      const newPendingFiles = filesToAdd.map((file) => createPendingFile(file));
      setPendingFiles((prev) => [...prev, ...newPendingFiles]);

      for (const pending of newPendingFiles) {
        try {
          const result = await uploadFile(pending.file, sessionId);
          setPendingFiles((prev) =>
            prev.map((f) =>
              f.id === pending.id
                ? {
                    ...f,
                    status: "ready" as const,
                    uploadedFileId: result.id,
                    uploadedFilename: result.filename ?? pending.file.name,
                  }
                : f,
            ),
          );
        } catch (error) {
          setPendingFiles((prev) =>
            prev.map((f) =>
              f.id === pending.id
                ? {
                    ...f,
                    status: "error" as const,
                    error: error instanceof Error ? error.message : "Upload failed",
                  }
                : f,
            ),
          );
        }
      }
    },
    [pendingFiles.length, maxFiles, sessionId],
  );

  const removeFile = useCallback((id: string) => {
    setPendingFiles((prev) => prev.filter((f) => f.id !== id));
  }, []);

  const clearFiles = useCallback(() => {
    cleanupPendingFiles(pendingFiles);
    setPendingFiles([]);
  }, [pendingFiles]);

  const handlePaste = useCallback(
    (event: React.ClipboardEvent | ClipboardEvent) => {
      const items = event.clipboardData?.items;
      if (!items) return;

      const pdfFiles: File[] = [];
      for (const item of Array.from(items)) {
        if (item.type === "application/pdf") {
          const file = item.getAsFile();
          if (file) {
            pdfFiles.push(file);
          }
        }
      }

      if (pdfFiles.length > 0) {
        event.preventDefault();
        addFiles(pdfFiles);
      }
    },
    [addFiles],
  );

  const handleDragOver = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    event.stopPropagation();
  }, []);

  const handleDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      event.stopPropagation();

      const files = event.dataTransfer?.files;
      if (!files || files.length === 0) return;

      const pdfFiles: File[] = [];
      for (const file of Array.from(files)) {
        if (file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf")) {
          pdfFiles.push(file);
        }
      }

      if (pdfFiles.length > 0) {
        addFiles(pdfFiles);
      }
    },
    [addFiles],
  );

  return {
    pendingFiles,
    allUploaded,
    uploadedFileIds,
    addFiles,
    removeFile,
    clearFiles,
    handlePaste,
    handleDragOver,
    handleDrop,
    hasFiles: pendingFiles.length > 0,
    isUploading: pendingFiles.some((f) => f.status === "uploading"),
  };
}
