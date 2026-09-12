import { getApiBaseUrl, throwApiError } from "./client";
import {
  ALLOWED_FILE_EXTENSIONS,
  ALLOWED_FILE_TYPES,
  MAX_FILE_SIZE,
  type FileUploadResponse,
} from "./legacy-api-types";

export { ALLOWED_FILE_EXTENSIONS, ALLOWED_FILE_TYPES, MAX_FILE_SIZE };
export type { FileUploadResponse };

export function validateFileUpload(file: File): string | null {
  const ext = "." + (file.name.split(".").pop() ?? "").toLowerCase();
  const mime = file.type || (ext === ".pdf" ? "application/pdf" : "");
  if (
    !(ALLOWED_FILE_EXTENSIONS as readonly string[]).includes(ext) &&
    !(ALLOWED_FILE_TYPES as readonly string[]).includes(mime)
  ) {
    return `Unsupported file type: ${file.name}. Only PDF files are supported.`;
  }
  if (file.size > MAX_FILE_SIZE) {
    return `File too large: ${file.name}. Maximum size is 32 MB.`;
  }
  return null;
}

export async function uploadFile(file: File, sessionId?: string): Promise<FileUploadResponse> {
  const formData = new FormData();
  formData.append("file", file, file.name);
  const params = sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : "";
  const baseUrl = getApiBaseUrl();
  const url = `${baseUrl}/v1/files${params}`;

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

export function getFileUrl(fileId: string): string {
  return `${getApiBaseUrl()}/v1/files/${encodeURIComponent(fileId)}`;
}

export interface PendingFile {
  id: string;
  file: File;
  status: "uploading" | "ready" | "error";
  error?: string;
  uploadedFileId?: string;
  uploadedFilename?: string;
}

export function createPendingFile(file: File): PendingFile {
  return {
    id: `${Date.now()}-${Math.random().toString(36).slice(2)}`,
    file,
    status: "uploading",
  };
}

export function cleanupPendingFiles(_files: PendingFile[]): void {
  // No object URLs are created for PDFs (no thumbnails); nothing to revoke.
}
