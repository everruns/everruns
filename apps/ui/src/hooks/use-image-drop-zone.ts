// Image drag-drop and paste handling hook for chat composer
//
// Decision: Extracts drag-drop and paste event handlers from ChatPanel into a reusable hook.
// Returns event handlers and drag state — the consumer wires them to the DOM elements.

import { useState, useCallback } from "react";

interface UseImageDropZoneOptions {
  /** Callback to handle dropped/pasted image files */
  onImageFiles: (files: File[]) => void;
}

export function useImageDropZone({ onImageFiles }: UseImageDropZoneOptions) {
  const [isDraggingOver, setIsDraggingOver] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDraggingOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (!e.currentTarget.contains(e.relatedTarget as Node)) {
      setIsDraggingOver(false);
    }
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDraggingOver(false);
      const files = e.dataTransfer?.files;
      if (files && files.length > 0) {
        const imageFiles = Array.from(files).filter((f) => f.type.startsWith("image/"));
        if (imageFiles.length > 0) {
          onImageFiles(imageFiles);
        }
      }
    },
    [onImageFiles],
  );

  const handlePaste = useCallback(
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      const imageFiles: File[] = [];
      for (const item of Array.from(items)) {
        if (item.type.startsWith("image/")) {
          const file = item.getAsFile();
          if (file) imageFiles.push(file);
        }
      }
      if (imageFiles.length > 0) {
        e.preventDefault();
        onImageFiles(imageFiles);
      }
    },
    [onImageFiles],
  );

  return {
    isDraggingOver,
    dropZoneProps: {
      onDragOver: handleDragOver,
      onDragEnter: handleDragEnter,
      onDragLeave: handleDragLeave,
      onDrop: handleDrop,
    },
    handlePaste,
  };
}
