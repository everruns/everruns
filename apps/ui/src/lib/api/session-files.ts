// Session Files (Virtual Filesystem) API functions
//
// RESTful API design:
// - GET    /fs/{path}  - Read file or list directory
// - GET    /fs/{path}?stat=true  - Get file metadata
// - POST   /fs/{path}  - Create file or directory
// - PUT    /fs/{path}  - Update file
// - DELETE /fs/{path}  - Delete file or directory
// - POST   /fs/_actions/move - Move/rename
// - POST   /fs/_actions/copy - Copy
// - POST   /fs/_actions/grep - Search

import { api } from "./client";
import type {
  FileInfo,
  SessionFile,
  FileStat,
  GrepResult,
  CreateFileRequest,
  UpdateFileRequest,
  MoveFileRequest,
  CopyFileRequest,
  GrepRequest,
  DeleteFileResponse,
  ListResponse,
} from "./types";

// Base path for filesystem
function fsPath(agentId: string, sessionId: string, path?: string): string {
  const base = `/v1/agents/${agentId}/sessions/${sessionId}/fs`;
  if (!path || path === "/") return base;
  // Ensure path doesn't have double slashes
  const normalizedPath = path.startsWith("/") ? path.slice(1) : path;
  return `${base}/${normalizedPath}`;
}

// ============================================
// File CRUD Operations
// ============================================

/** List files in a directory */
export async function listFiles(
  agentId: string,
  sessionId: string,
  path: string = "/",
  recursive: boolean = false
): Promise<FileInfo[]> {
  const url = recursive
    ? `${fsPath(agentId, sessionId, path)}?recursive=true`
    : fsPath(agentId, sessionId, path);
  const response = await api.get<ListResponse<FileInfo>>(url);
  return response.data.data;
}

/** Create a new file */
export async function createFile(
  agentId: string,
  sessionId: string,
  request: CreateFileRequest
): Promise<SessionFile> {
  const { path, ...body } = request;
  const response = await api.post<SessionFile>(
    fsPath(agentId, sessionId, path),
    body
  );
  return response.data;
}

/** Read a file */
export async function readFile(
  agentId: string,
  sessionId: string,
  path: string
): Promise<SessionFile> {
  const response = await api.get<SessionFile>(
    fsPath(agentId, sessionId, path)
  );
  return response.data;
}

/** Update a file */
export async function updateFile(
  agentId: string,
  sessionId: string,
  path: string,
  request: UpdateFileRequest
): Promise<SessionFile> {
  const response = await api.put<SessionFile>(
    fsPath(agentId, sessionId, path),
    request
  );
  return response.data;
}

/** Get file stat (metadata) */
export async function statFile(
  agentId: string,
  sessionId: string,
  path: string
): Promise<FileStat> {
  const url = `${fsPath(agentId, sessionId, path)}?stat=true`;
  const response = await api.get<FileStat>(url);
  return response.data;
}

/** Delete a file or directory */
export async function deleteFile(
  agentId: string,
  sessionId: string,
  path: string,
  recursive: boolean = false
): Promise<boolean> {
  const url = recursive
    ? `${fsPath(agentId, sessionId, path)}?recursive=true`
    : fsPath(agentId, sessionId, path);
  const response = await api.delete<DeleteFileResponse>(url);
  return response.data.deleted;
}

// ============================================
// Directory Operations
// ============================================

/** Create a directory */
export async function mkdir(
  agentId: string,
  sessionId: string,
  path: string
): Promise<SessionFile> {
  const response = await api.post<SessionFile>(
    fsPath(agentId, sessionId, path),
    { is_directory: true }
  );
  return response.data;
}

// ============================================
// File Management Operations
// ============================================

/** Move/rename a file or directory */
export async function moveFile(
  agentId: string,
  sessionId: string,
  request: MoveFileRequest
): Promise<SessionFile> {
  const response = await api.post<SessionFile>(
    `/v1/agents/${agentId}/sessions/${sessionId}/fs/_actions/move`,
    request
  );
  return response.data;
}

/** Copy a file */
export async function copyFile(
  agentId: string,
  sessionId: string,
  request: CopyFileRequest
): Promise<SessionFile> {
  const response = await api.post<SessionFile>(
    `/v1/agents/${agentId}/sessions/${sessionId}/fs/_actions/copy`,
    request
  );
  return response.data;
}

// ============================================
// Search Operations
// ============================================

/** Search files using grep-like pattern matching */
export async function grepFiles(
  agentId: string,
  sessionId: string,
  request: GrepRequest
): Promise<GrepResult[]> {
  const response = await api.post<ListResponse<GrepResult>>(
    `/v1/agents/${agentId}/sessions/${sessionId}/fs/_actions/grep`,
    request
  );
  return response.data.data;
}

// ============================================
// Utility Functions
// ============================================

/** Format file size in human-readable format */
export function formatFileSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${Number.parseFloat((bytes / k ** i).toFixed(1))} ${sizes[i]}`;
}

/** Get file extension from path */
export function getFileExtension(path: string): string {
  const name = path.split("/").pop() ?? "";
  const ext = name.split(".").pop();
  return ext && ext !== name ? ext : "";
}

/** Get parent directory path */
export function getParentPath(path: string): string | null {
  if (path === "/") return null;
  const parts = path.split("/").filter(Boolean);
  parts.pop();
  return parts.length === 0 ? "/" : `/${parts.join("/")}`;
}

/** Join path segments */
export function joinPath(...segments: string[]): string {
  const path = segments
    .filter(Boolean)
    .join("/")
    .replace(/\/+/g, "/");
  return path.startsWith("/") ? path : `/${path}`;
}
