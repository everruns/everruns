// Session Files (Virtual Filesystem) API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)
//
// Target endpoints are the canonical Workspace filesystem surface
// (/v1/workspaces/{workspace_id}/fs/*). The hooks/components keep passing a
// `sessionId` because every UI flow today operates on the default per-session
// Workspace; the API client derives the workspace public ID from it via
// `workspaceIdFromSessionId` (the migration backs each session with a default
// Workspace whose UUID equals the session's UUID, so the hex portion of the
// public IDs is identical). Move/copy and download still live on the legacy
// session routes — those are not exposed through the workspace surface yet.

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

// Derive the default Workspace public ID for a Session. The server creates a
// 1:1 default Workspace with the same UUID per session, so the public IDs
// share the same 32-hex suffix and only differ by prefix.
function workspaceIdFromSessionId(sessionId: string): string {
  return sessionId.startsWith("session_") ? `wsp_${sessionId.slice("session_".length)}` : sessionId;
}

// Base path for the workspace filesystem.
function fsPath(sessionId: string, path?: string): string {
  const workspaceId = workspaceIdFromSessionId(sessionId);
  const base = `/v1/workspaces/${workspaceId}/fs`;
  if (!path || path === "/") return base;
  const normalizedPath = path.startsWith("/") ? path.slice(1) : path;
  return `${base}/${normalizedPath}`;
}

function actionPath(sessionId: string, action: string): string {
  const workspaceId = workspaceIdFromSessionId(sessionId);
  return `/v1/workspaces/${workspaceId}/fs/_/${action}`;
}

// Legacy session-scoped path: used only for move/copy which the workspace
// surface does not yet expose.
function legacySessionActionPath(sessionId: string, action: string): string {
  return `/v1/sessions/${sessionId}/fs/_/${action}`;
}

// ============================================
// File CRUD Operations
// ============================================

/** List files in a directory */
export async function listFiles(
  sessionId: string,
  path: string = "/",
  recursive: boolean = false,
): Promise<FileInfo[]> {
  const url = recursive ? `${fsPath(sessionId, path)}?recursive=true` : fsPath(sessionId, path);
  const response = await api.get<ListResponse<FileInfo>>(url);
  return response.data.data;
}

/** Create a new file */
export async function createFile(
  sessionId: string,
  request: CreateFileRequest,
): Promise<SessionFile> {
  const { path, ...body } = request;
  const response = await api.post<SessionFile>(fsPath(sessionId, path), body);
  return response.data;
}

/** Read a file */
export async function readFile(sessionId: string, path: string): Promise<SessionFile> {
  const response = await api.get<SessionFile>(fsPath(sessionId, path));
  return response.data;
}

/** Update a file */
export async function updateFile(
  sessionId: string,
  path: string,
  request: UpdateFileRequest,
): Promise<SessionFile> {
  const response = await api.put<SessionFile>(fsPath(sessionId, path), request);
  return response.data;
}

/** Get file stat (metadata) */
export async function statFile(sessionId: string, path: string): Promise<FileStat> {
  const response = await api.post<FileStat>(actionPath(sessionId, "stat"), { path });
  return response.data;
}

/** Delete a file or directory */
export async function deleteFile(
  sessionId: string,
  path: string,
  recursive: boolean = false,
): Promise<boolean> {
  const url = recursive ? `${fsPath(sessionId, path)}?recursive=true` : fsPath(sessionId, path);
  const response = await api.delete<DeleteFileResponse>(url);
  return response.data.deleted;
}

// ============================================
// Directory Operations
// ============================================

/** Create a directory */
export async function mkdir(sessionId: string, path: string): Promise<SessionFile> {
  const response = await api.post<SessionFile>(fsPath(sessionId, path), { is_directory: true });
  return response.data;
}

// ============================================
// File Management Operations
// ============================================

/** Move/rename a file or directory */
export async function moveFile(sessionId: string, request: MoveFileRequest): Promise<SessionFile> {
  const response = await api.post<SessionFile>(legacySessionActionPath(sessionId, "move"), request);
  return response.data;
}

/** Copy a file */
export async function copyFile(sessionId: string, request: CopyFileRequest): Promise<SessionFile> {
  const response = await api.post<SessionFile>(legacySessionActionPath(sessionId, "copy"), request);
  return response.data;
}

// ============================================
// Search Operations
// ============================================

/** Search files using grep-like pattern matching */
export async function grepFiles(sessionId: string, request: GrepRequest): Promise<GrepResult[]> {
  const response = await api.post<ListResponse<GrepResult>>(actionPath(sessionId, "grep"), request);
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
