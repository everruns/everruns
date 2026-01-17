// Session Files hooks for virtual filesystem operations
"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  listFiles,
  createFile,
  readFile,
  updateFile,
  statFile,
  deleteFile,
  mkdir,
  moveFile,
  copyFile,
  grepFiles,
} from "@/lib/api/session-files";
import type {
  CreateFileRequest,
  UpdateFileRequest,
  MoveFileRequest,
  CopyFileRequest,
  GrepRequest,
} from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

// Query key factory
const fileKeys = {
  all: (org: string, agentId: string, sessionId: string) => ["files", org, agentId, sessionId] as const,
  list: (org: string, agentId: string, sessionId: string, path: string, recursive: boolean) =>
    [...fileKeys.all(org, agentId, sessionId), "list", path, recursive] as const,
  file: (org: string, agentId: string, sessionId: string, path: string) =>
    [...fileKeys.all(org, agentId, sessionId), "file", path] as const,
  stat: (org: string, agentId: string, sessionId: string, path: string) =>
    [...fileKeys.all(org, agentId, sessionId), "stat", path] as const,
};

// ============================================
// Query Hooks
// ============================================

/** List files in a directory */
export function useFiles(
  agentId: string | undefined,
  sessionId: string | undefined,
  path: string = "/",
  recursive: boolean = false
) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: fileKeys.list(org!, agentId!, sessionId!, path, recursive),
    queryFn: () => listFiles(org!, agentId!, sessionId!, path, recursive),
    enabled: !!org && !!agentId && !!sessionId,
  });
}

/** Read a file */
export function useFile(
  agentId: string | undefined,
  sessionId: string | undefined,
  path: string | undefined
) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: fileKeys.file(org!, agentId!, sessionId!, path!),
    queryFn: () => readFile(org!, agentId!, sessionId!, path!),
    enabled: !!org && !!agentId && !!sessionId && !!path,
  });
}

/** Get file stat */
export function useFileStat(
  agentId: string | undefined,
  sessionId: string | undefined,
  path: string | undefined
) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: fileKeys.stat(org!, agentId!, sessionId!, path!),
    queryFn: () => statFile(org!, agentId!, sessionId!, path!),
    enabled: !!org && !!agentId && !!sessionId && !!path,
  });
}

// ============================================
// Mutation Hooks
// ============================================

/** Create a file */
export function useCreateFile() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: CreateFileRequest;
    }) => createFile(org!, agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, agentId, sessionId),
      });
    },
  });
}

/** Create a directory */
export function useCreateDirectory() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      path,
    }: {
      agentId: string;
      sessionId: string;
      path: string;
    }) => mkdir(org!, agentId, sessionId, path),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, agentId, sessionId),
      });
    },
  });
}

/** Update a file */
export function useUpdateFile() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      path,
      request,
    }: {
      agentId: string;
      sessionId: string;
      path: string;
      request: UpdateFileRequest;
    }) => updateFile(org!, agentId, sessionId, path, request),
    onSuccess: (_, { agentId, sessionId, path }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, agentId, sessionId),
      });
      queryClient.invalidateQueries({
        queryKey: fileKeys.file(org!, agentId, sessionId, path),
      });
    },
  });
}

/** Delete a file or directory */
export function useDeleteFile() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      path,
      recursive = false,
    }: {
      agentId: string;
      sessionId: string;
      path: string;
      recursive?: boolean;
    }) => deleteFile(org!, agentId, sessionId, path, recursive),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, agentId, sessionId),
      });
    },
  });
}

/** Move/rename a file */
export function useMoveFile() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: MoveFileRequest;
    }) => moveFile(org!, agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, agentId, sessionId),
      });
    },
  });
}

/** Copy a file */
export function useCopyFile() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: CopyFileRequest;
    }) => copyFile(org!, agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, agentId, sessionId),
      });
    },
  });
}

/** Search files using grep */
export function useGrepFiles() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: GrepRequest;
    }) => grepFiles(org!, agentId, sessionId, request),
  });
}
