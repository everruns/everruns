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
  all: (org: string, sessionId: string) => ["files", org, sessionId] as const,
  list: (org: string, sessionId: string, path: string, recursive: boolean) =>
    [...fileKeys.all(org, sessionId), "list", path, recursive] as const,
  file: (org: string, sessionId: string, path: string) =>
    [...fileKeys.all(org, sessionId), "file", path] as const,
  stat: (org: string, sessionId: string, path: string) =>
    [...fileKeys.all(org, sessionId), "stat", path] as const,
};

// ============================================
// Query Hooks
// ============================================

/** List files in a directory */
export function useFiles(
  sessionId: string | undefined,
  path: string = "/",
  recursive: boolean = false,
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: fileKeys.list(org!, sessionId!, path, recursive),
    queryFn: () => listFiles(sessionId!, path, recursive),
    enabled: !!org && !!sessionId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Read a file */
export function useFile(sessionId: string | undefined, path: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: fileKeys.file(org!, sessionId!, path!),
    queryFn: () => readFile(sessionId!, path!),
    enabled: !!org && !!sessionId && !!path,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Get file stat */
export function useFileStat(sessionId: string | undefined, path: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: fileKeys.stat(org!, sessionId!, path!),
    queryFn: () => statFile(sessionId!, path!),
    enabled: !!org && !!sessionId && !!path,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
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
    mutationFn: ({ sessionId, request }: { sessionId: string; request: CreateFileRequest }) =>
      createFile(sessionId, request),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, sessionId),
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
    mutationFn: ({ sessionId, path }: { sessionId: string; path: string }) =>
      mkdir(sessionId, path),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, sessionId),
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
      sessionId,
      path,
      request,
    }: {
      sessionId: string;
      path: string;
      request: UpdateFileRequest;
    }) => updateFile(sessionId, path, request),
    onSuccess: (_, { sessionId, path }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, sessionId),
      });
      queryClient.invalidateQueries({
        queryKey: fileKeys.file(org!, sessionId, path),
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
      sessionId,
      path,
      recursive = false,
    }: {
      sessionId: string;
      path: string;
      recursive?: boolean;
    }) => deleteFile(sessionId, path, recursive),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, sessionId),
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
    mutationFn: ({ sessionId, request }: { sessionId: string; request: MoveFileRequest }) =>
      moveFile(sessionId, request),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, sessionId),
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
    mutationFn: ({ sessionId, request }: { sessionId: string; request: CopyFileRequest }) =>
      copyFile(sessionId, request),
    onSuccess: (_, { sessionId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, sessionId),
      });
    },
  });
}

/** Search files using grep */
export function useGrepFiles() {
  return useMutation({
    mutationFn: ({ sessionId, request }: { sessionId: string; request: GrepRequest }) =>
      grepFiles(sessionId, request),
  });
}
