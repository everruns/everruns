// Image upload API
// All routes are org-scoped: /v1/orgs/{org}/images/...
//
// API functions for uploading, retrieving, and deleting images.
// Images can be attached to messages as image_file content parts.

import { getApiBaseUrl } from "./client";
import type { ImageUploadResponse, ImageInfo } from "./types";
import { ALLOWED_IMAGE_TYPES, MAX_IMAGE_SIZE } from "./types";

/**
 * Validate that a file is an allowed image type and size
 */
export function validateImageFile(file: File): { valid: boolean; error?: string } {
  // Check file type
  if (!ALLOWED_IMAGE_TYPES.includes(file.type as typeof ALLOWED_IMAGE_TYPES[number])) {
    return {
      valid: false,
      error: `Invalid file type "${file.type}". Allowed: ${ALLOWED_IMAGE_TYPES.join(", ")}`,
    };
  }

  // Check file size
  if (file.size > MAX_IMAGE_SIZE) {
    return {
      valid: false,
      error: `File too large (${formatBytes(file.size)}). Maximum: ${formatBytes(MAX_IMAGE_SIZE)}`,
    };
  }

  return { valid: true };
}

/**
 * Format bytes as human-readable string
 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Upload an image file
 */
export async function uploadImage(
  org: string,
  file: File,
  sessionId?: string
): Promise<ImageUploadResponse> {
  const formData = new FormData();
  formData.append("file", file);

  let url = `${getApiBaseUrl()}/v1/orgs/${org}/images`;
  if (sessionId) {
    url += `?session_id=${sessionId}`;
  }

  const response = await fetch(url, {
    method: "POST",
    body: formData,
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Failed to upload image: ${text}`);
  }

  return response.json();
}

/**
 * Get image URL for display (original size)
 */
export function getImageUrl(org: string, imageId: string): string {
  return `${getApiBaseUrl()}/v1/orgs/${org}/images/${imageId}`;
}

/**
 * Get thumbnail URL for display
 */
export function getThumbnailUrl(org: string, imageId: string): string {
  return `${getApiBaseUrl()}/v1/orgs/${org}/images/${imageId}/thumbnail`;
}

/**
 * Delete an image
 */
export async function deleteImage(org: string, imageId: string): Promise<void> {
  const response = await fetch(`${getApiBaseUrl()}/v1/orgs/${org}/images/${imageId}`, {
    method: "DELETE",
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Failed to delete image: ${text}`);
  }
}

/**
 * List images
 */
export async function listImages(
  org: string,
  limit: number = 50,
  offset: number = 0
): Promise<ImageInfo[]> {
  const response = await fetch(
    `${getApiBaseUrl()}/v1/orgs/${org}/images?limit=${limit}&offset=${offset}`
  );

  if (!response.ok) {
    const text = await response.text();
    throw new Error(`Failed to list images: ${text}`);
  }

  return response.json();
}

/**
 * Represents an image being uploaded or already uploaded
 */
export interface PendingImage {
  /** Temporary ID for tracking in UI */
  tempId: string;
  /** File being uploaded (null if already uploaded) */
  file: File | null;
  /** Upload promise (null if already resolved) */
  uploadPromise: Promise<ImageUploadResponse> | null;
  /** Image ID once uploaded */
  imageId: string | null;
  /** Original filename */
  filename: string;
  /** Preview URL (object URL or thumbnail URL) */
  previewUrl: string;
  /** Upload status */
  status: "uploading" | "uploaded" | "error";
  /** Error message if status is "error" */
  error?: string;
}

/**
 * Create a pending image from a file
 */
export function createPendingImage(org: string, file: File, sessionId?: string): PendingImage {
  const tempId = crypto.randomUUID();
  const previewUrl = URL.createObjectURL(file);

  const uploadPromise = uploadImage(org, file, sessionId);

  return {
    tempId,
    file,
    uploadPromise,
    imageId: null,
    filename: file.name,
    previewUrl,
    status: "uploading",
  };
}

/**
 * Revoke object URLs for pending images when done
 */
export function cleanupPendingImages(images: PendingImage[]): void {
  for (const img of images) {
    if (img.file && img.previewUrl.startsWith("blob:")) {
      URL.revokeObjectURL(img.previewUrl);
    }
  }
}
