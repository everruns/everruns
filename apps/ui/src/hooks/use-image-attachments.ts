"use client";

import { useState, useCallback, useEffect } from "react";
import type { PendingImage } from "@/lib/api/images";
import {
  createPendingImage,
  cleanupPendingImages,
  validateImageFile,
} from "@/lib/api/images";

interface UseImageAttachmentsOptions {
  sessionId?: string;
  maxImages?: number;
}

/**
 * Hook to manage image attachments for chat input
 *
 * Handles:
 * - Adding images (from files or paste)
 * - Uploading images in parallel
 * - Tracking upload status
 * - Removing images
 * - Cleanup on unmount
 */
export function useImageAttachments(options: UseImageAttachmentsOptions = {}) {
  const { sessionId, maxImages = 10 } = options;
  const [pendingImages, setPendingImages] = useState<PendingImage[]>([]);

  // Track if all images are uploaded
  const allUploaded = pendingImages.every(
    (img) => img.status === "uploaded" || img.status === "error"
  );

  // Get IDs of successfully uploaded images
  const uploadedImageIds = pendingImages
    .filter((img) => img.status === "uploaded" && img.imageId)
    .map((img) => ({ imageId: img.imageId!, filename: img.filename }));

  // Cleanup object URLs on unmount
  useEffect(() => {
    return () => {
      cleanupPendingImages(pendingImages);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /**
   * Add files to the pending images list and start uploading
   */
  const addFiles = useCallback(
    async (files: File[]) => {
      // Filter to only valid files
      const validFiles: File[] = [];
      const errors: string[] = [];

      for (const file of files) {
        const validation = validateImageFile(file);
        if (validation.valid) {
          validFiles.push(file);
        } else {
          errors.push(`${file.name}: ${validation.error}`);
        }
      }

      // Check max images
      const remainingSlots = maxImages - pendingImages.length;
      const filesToAdd = validFiles.slice(0, remainingSlots);

      if (filesToAdd.length < validFiles.length) {
        errors.push(`Only ${maxImages} images allowed. Some images were not added.`);
      }

      if (errors.length > 0) {
        // Log errors but don't block valid files
        console.warn("Image validation errors:", errors);
      }

      if (filesToAdd.length === 0) return;

      // Create pending images and start uploads
      const newPendingImages = filesToAdd.map((file) =>
        createPendingImage(file, sessionId)
      );

      // Add to state
      setPendingImages((prev) => [...prev, ...newPendingImages]);

      // Process upload results
      for (const pendingImage of newPendingImages) {
        if (!pendingImage.uploadPromise) continue;

        pendingImage.uploadPromise
          .then((result) => {
            setPendingImages((prev) =>
              prev.map((img) =>
                img.tempId === pendingImage.tempId
                  ? {
                      ...img,
                      status: "uploaded" as const,
                      imageId: result.id,
                      uploadPromise: null,
                    }
                  : img
              )
            );
          })
          .catch((error) => {
            setPendingImages((prev) =>
              prev.map((img) =>
                img.tempId === pendingImage.tempId
                  ? {
                      ...img,
                      status: "error" as const,
                      error: error instanceof Error ? error.message : "Upload failed",
                      uploadPromise: null,
                    }
                  : img
              )
            );
          });
      }
    },
    [pendingImages.length, maxImages, sessionId]
  );

  /**
   * Remove an image from the list
   */
  const removeImage = useCallback((tempId: string) => {
    setPendingImages((prev) => {
      const image = prev.find((img) => img.tempId === tempId);
      if (image && image.file && image.previewUrl.startsWith("blob:")) {
        URL.revokeObjectURL(image.previewUrl);
      }
      return prev.filter((img) => img.tempId !== tempId);
    });
  }, []);

  /**
   * Clear all images
   */
  const clearImages = useCallback(() => {
    cleanupPendingImages(pendingImages);
    setPendingImages([]);
  }, [pendingImages]);

  /**
   * Handle paste event - extract images from clipboard
   */
  const handlePaste = useCallback(
    (event: React.ClipboardEvent | ClipboardEvent) => {
      const items = event.clipboardData?.items;
      if (!items) return;

      const imageFiles: File[] = [];
      for (const item of Array.from(items)) {
        if (item.type.startsWith("image/")) {
          const file = item.getAsFile();
          if (file) {
            imageFiles.push(file);
          }
        }
      }

      if (imageFiles.length > 0) {
        addFiles(imageFiles);
      }
    },
    [addFiles]
  );

  return {
    pendingImages,
    allUploaded,
    uploadedImageIds,
    addFiles,
    removeImage,
    clearImages,
    handlePaste,
    hasImages: pendingImages.length > 0,
    isUploading: pendingImages.some((img) => img.status === "uploading"),
  };
}
