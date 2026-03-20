// Image upload API
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)
//
// API functions for uploading, retrieving, and deleting images.
// Images can be attached to messages as image_file content parts.

import { api, getApiBaseUrl, throwApiError } from "./client";
import type { ImageUploadResponse, ImageInfo } from "./types";
import { ALLOWED_IMAGE_TYPES, MAX_IMAGE_SIZE } from "./types";

/**
 * Validate that a file is an allowed image type and size
 */
export function validateImageFile(file: File): { valid: boolean; error?: string } {
  // Check file type
  if (!ALLOWED_IMAGE_TYPES.includes(file.type as (typeof ALLOWED_IMAGE_TYPES)[number])) {
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
 * @param file - Image file to upload
 * @param sessionId - Optional: session ID stored as metadata for tracking (not required)
 */
export async function uploadImage(file: File, sessionId?: string): Promise<ImageUploadResponse> {
  const formData = new FormData();
  formData.append("file", file);

  const baseUrl = getApiBaseUrl();
  const params = sessionId ? `?session_id=${sessionId}` : "";
  const url = `${baseUrl}/v1/images${params}`;

  // Raw fetch needed for FormData (no Content-Type header — browser sets multipart boundary)
  const response = await fetch(url, {
    method: "POST",
    body: formData,
    credentials: "include",
  });

  if (!response.ok) {
    await throwApiError(response);
  }

  return response.json();
}

/**
 * Get image URL for display (original size)
 */
export function getImageUrl(imageId: string): string {
  return `${getApiBaseUrl()}/v1/images/${imageId}`;
}

/**
 * Get thumbnail URL for display
 */
export function getThumbnailUrl(imageId: string): string {
  return `${getApiBaseUrl()}/v1/images/${imageId}/thumbnail`;
}

/**
 * Delete an image
 */
export async function deleteImage(imageId: string): Promise<void> {
  await api.delete(`/v1/images/${imageId}`);
}

/**
 * List images
 */
export async function listImages(limit: number = 50, offset: number = 0): Promise<ImageInfo[]> {
  const response = await api.get<ImageInfo[]>(`/v1/images?limit=${limit}&offset=${offset}`);
  return response.data;
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
 * @param file - Image file to upload
 * @param sessionId - Optional: session ID stored as metadata for tracking (not required)
 */
export function createPendingImage(file: File, sessionId?: string): PendingImage {
  const tempId = crypto.randomUUID();
  const previewUrl = URL.createObjectURL(file);

  const uploadPromise = uploadImage(file, sessionId);

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
