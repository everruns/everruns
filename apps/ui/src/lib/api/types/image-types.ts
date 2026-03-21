// Image types (for message attachments)

/** Image metadata (returned from upload) */
export interface ImageInfo {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  metadata: Record<string, unknown>;
  created_at: string;
}

/** Image upload response */
export interface ImageUploadResponse {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  created_at: string;
}

/** Allowed image content types */
export const ALLOWED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;

export type AllowedImageType = (typeof ALLOWED_IMAGE_TYPES)[number];

/** Maximum image size in bytes (100 MB) */
export const MAX_IMAGE_SIZE = 100 * 1024 * 1024;
