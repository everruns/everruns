// Session Files (Virtual Filesystem) API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)
//
// All file operations target the canonical Workspace filesystem surface
// (/v1/workspaces/{workspace_id}/fs/*). Callers pass the session's attached
// `workspace_id` (from the session response) rather than deriving it from the
// session id — the derivation only holds under the default 1:1 invariant and
// breaks for a session attached to a shared Workspace.

import { api, getApiBaseUrl } from "./client";
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

// Base path for the session's virtual filesystem.
// Each path segment is percent-encoded so that URL delimiter characters
// (?, #, %) in filenames are treated as literal bytes, not as query-string or
// fragment delimiters. E.g. a file named "foo?x=1" becomes "foo%3Fx%3D1" in the
// URL and reaches the server as a literal path.
function fsPath(workspaceId: string, path?: string): string {
  const base = `/v1/workspaces/${workspaceId}/fs`;
  if (!path || path === "/") return base;
  const normalizedPath = path.startsWith("/") ? path.slice(1) : path;
  const encodedPath = normalizedPath.split("/").map(encodeURIComponent).join("/");
  return `${base}/${encodedPath}`;
}

function actionPath(workspaceId: string, action: string): string {
  return `/v1/workspaces/${workspaceId}/fs/_/${action}`;
}

/**
 * Browser-navigable URL for the sandboxed HTML preview endpoint (TM-WEB-010).
 *
 * Used as an `<iframe src>`, so it includes the API base prefix and is loaded
 * directly by the browser (auth cookies ride along same-origin). The server
 * responds with a strict `sandbox` CSP, isolating the rendered document from
 * everruns. Each path segment is percent-encoded; `/` stays a separator to match
 * the `{*path}` wildcard route.
 */
export function htmlPreviewUrl(workspaceId: string, path: string): string {
  const normalizedPath = path.startsWith("/") ? path.slice(1) : path;
  const encodedPath = normalizedPath.split("/").map(encodeURIComponent).join("/");
  return `${getApiBaseUrl()}/v1/workspaces/${workspaceId}/fs/_/preview/${encodedPath}`;
}

// ============================================
// File CRUD Operations
// ============================================

/** List files in a directory */
export async function listFiles(
  workspaceId: string,
  path: string = "/",
  recursive: boolean = false,
): Promise<FileInfo[]> {
  const base = fsPath(workspaceId, path);
  const url = recursive ? `${base}?${new URLSearchParams({ recursive: "true" })}` : base;
  const response = await api.get<ListResponse<FileInfo>>(url);
  return response.data.data;
}

/** Create a new file */
export async function createFile(
  workspaceId: string,
  request: CreateFileRequest,
): Promise<SessionFile> {
  const { path, ...body } = request;
  const response = await api.post<SessionFile>(fsPath(workspaceId, path), body);
  return response.data;
}

/** Read a file */
export async function readFile(workspaceId: string, path: string): Promise<SessionFile> {
  const response = await api.get<SessionFile>(fsPath(workspaceId, path));
  return response.data;
}

/** Update a file */
export async function updateFile(
  workspaceId: string,
  path: string,
  request: UpdateFileRequest,
): Promise<SessionFile> {
  const response = await api.put<SessionFile>(fsPath(workspaceId, path), request);
  return response.data;
}

/** Get file stat (metadata) */
export async function statFile(workspaceId: string, path: string): Promise<FileStat> {
  const response = await api.post<FileStat>(actionPath(workspaceId, "stat"), { path });
  return response.data;
}

/** Delete a file or directory */
export async function deleteFile(
  workspaceId: string,
  path: string,
  recursive: boolean = false,
): Promise<boolean> {
  const base = fsPath(workspaceId, path);
  const url = recursive ? `${base}?${new URLSearchParams({ recursive: "true" })}` : base;
  const response = await api.delete<DeleteFileResponse>(url);
  return response.data.deleted;
}

// ============================================
// Directory Operations
// ============================================

/** Create a directory */
export async function mkdir(workspaceId: string, path: string): Promise<SessionFile> {
  const response = await api.post<SessionFile>(fsPath(workspaceId, path), { is_directory: true });
  return response.data;
}

// ============================================
// File Management Operations
// ============================================

/** Move/rename a file or directory */
export async function moveFile(
  workspaceId: string,
  request: MoveFileRequest,
): Promise<SessionFile> {
  const response = await api.post<SessionFile>(actionPath(workspaceId, "move"), request);
  return response.data;
}

/** Copy a file */
export async function copyFile(
  workspaceId: string,
  request: CopyFileRequest,
): Promise<SessionFile> {
  const response = await api.post<SessionFile>(actionPath(workspaceId, "copy"), request);
  return response.data;
}

// ============================================
// Search Operations
// ============================================

/** Search files using grep-like pattern matching */
export async function grepFiles(workspaceId: string, request: GrepRequest): Promise<GrepResult[]> {
  const response = await api.post<ListResponse<GrepResult>>(
    actionPath(workspaceId, "grep"),
    request,
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
  const path = segments.filter(Boolean).join("/").replace(/\/+/g, "/");
  return path.startsWith("/") ? path : `/${path}`;
}
