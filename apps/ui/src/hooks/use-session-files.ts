// Session Files hooks for virtual filesystem operations

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

// Query key factory
const fileKeys = {
  all: (agentId: string, sessionId: string) => ["files", agentId, sessionId] as const,
  list: (agentId: string, sessionId: string, path: string, recursive: boolean) =>
    [...fileKeys.all(agentId, sessionId), "list", path, recursive] as const,
  file: (agentId: string, sessionId: string, path: string) =>
    [...fileKeys.all(agentId, sessionId), "file", path] as const,
  stat: (agentId: string, sessionId: string, path: string) =>
    [...fileKeys.all(agentId, sessionId), "stat", path] as const,
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
  return useQuery({
    queryKey: fileKeys.list(agentId!, sessionId!, path, recursive),
    queryFn: () => listFiles(agentId!, sessionId!, path, recursive),
    enabled: !!agentId && !!sessionId,
  });
}

/** Read a file */
export function useFile(
  agentId: string | undefined,
  sessionId: string | undefined,
  path: string | undefined
) {
  return useQuery({
    queryKey: fileKeys.file(agentId!, sessionId!, path!),
    queryFn: () => readFile(agentId!, sessionId!, path!),
    enabled: !!agentId && !!sessionId && !!path,
  });
}

/** Get file stat */
export function useFileStat(
  agentId: string | undefined,
  sessionId: string | undefined,
  path: string | undefined
) {
  return useQuery({
    queryKey: fileKeys.stat(agentId!, sessionId!, path!),
    queryFn: () => statFile(agentId!, sessionId!, path!),
    enabled: !!agentId && !!sessionId && !!path,
  });
}

// ============================================
// Mutation Hooks
// ============================================

/** Create a file */
export function useCreateFile() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: CreateFileRequest;
    }) => createFile(agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(agentId, sessionId),
      });
    },
  });
}

/** Create a directory */
export function useCreateDirectory() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      path,
    }: {
      agentId: string;
      sessionId: string;
      path: string;
    }) => mkdir(agentId, sessionId, path),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(agentId, sessionId),
      });
    },
  });
}

/** Update a file */
export function useUpdateFile() {
  const queryClient = useQueryClient();

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
    }) => updateFile(agentId, sessionId, path, request),
    onSuccess: (_, { agentId, sessionId, path }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(agentId, sessionId),
      });
      queryClient.invalidateQueries({
        queryKey: fileKeys.file(agentId, sessionId, path),
      });
    },
  });
}

/** Delete a file or directory */
export function useDeleteFile() {
  const queryClient = useQueryClient();

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
    }) => deleteFile(agentId, sessionId, path, recursive),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(agentId, sessionId),
      });
    },
  });
}

/** Move/rename a file */
export function useMoveFile() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: MoveFileRequest;
    }) => moveFile(agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(agentId, sessionId),
      });
    },
  });
}

/** Copy a file */
export function useCopyFile() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: CopyFileRequest;
    }) => copyFile(agentId, sessionId, request),
    onSuccess: (_, { agentId, sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(agentId, sessionId),
      });
    },
  });
}

/** Search files using grep */
export function useGrepFiles() {
  return useMutation({
    mutationFn: ({
      agentId,
      sessionId,
      request,
    }: {
      agentId: string;
      sessionId: string;
      request: GrepRequest;
    }) => grepFiles(agentId, sessionId, request),
  });
}
