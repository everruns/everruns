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
  all: (org: string, workspaceId: string) => ["files", org, workspaceId] as const,
  list: (org: string, workspaceId: string, path: string, recursive: boolean) =>
    [...fileKeys.all(org, workspaceId), "list", path, recursive] as const,
  file: (org: string, workspaceId: string, path: string) =>
    [...fileKeys.all(org, workspaceId), "file", path] as const,
  stat: (org: string, workspaceId: string, path: string) =>
    [...fileKeys.all(org, workspaceId), "stat", path] as const,
};

// ============================================
// Query Hooks
// ============================================

/** List files in a directory */
export function useFiles(
  workspaceId: string | undefined,
  path: string = "/",
  recursive: boolean = false,
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: fileKeys.list(org!, workspaceId!, path, recursive),
    queryFn: () => listFiles(workspaceId!, path, recursive),
    enabled: !!org && !!workspaceId,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Read a file */
export function useFile(workspaceId: string | undefined, path: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: fileKeys.file(org!, workspaceId!, path!),
    queryFn: () => readFile(workspaceId!, path!),
    enabled: !!org && !!workspaceId && !!path,
  });

  // Include org loading state so pages show skeleton while org initializes
  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/** Get file stat */
export function useFileStat(workspaceId: string | undefined, path: string | undefined) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: fileKeys.stat(org!, workspaceId!, path!),
    queryFn: () => statFile(workspaceId!, path!),
    enabled: !!org && !!workspaceId && !!path,
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
    mutationFn: ({ workspaceId, request }: { workspaceId: string; request: CreateFileRequest }) =>
      createFile(workspaceId, request),
    onSuccess: (_, { workspaceId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, workspaceId),
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
    mutationFn: ({ workspaceId, path }: { workspaceId: string; path: string }) =>
      mkdir(workspaceId, path),
    onSuccess: (_, { workspaceId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, workspaceId),
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
      workspaceId,
      path,
      request,
    }: {
      workspaceId: string;
      path: string;
      request: UpdateFileRequest;
    }) => updateFile(workspaceId, path, request),
    onSuccess: (_, { workspaceId, path }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, workspaceId),
      });
      queryClient.invalidateQueries({
        queryKey: fileKeys.file(org!, workspaceId, path),
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
      workspaceId,
      path,
      recursive = false,
    }: {
      workspaceId: string;
      path: string;
      recursive?: boolean;
    }) => deleteFile(workspaceId, path, recursive),
    onSuccess: (_, { workspaceId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, workspaceId),
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
    mutationFn: ({ workspaceId, request }: { workspaceId: string; request: MoveFileRequest }) =>
      moveFile(workspaceId, request),
    onSuccess: (_, { workspaceId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, workspaceId),
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
    mutationFn: ({ workspaceId, request }: { workspaceId: string; request: CopyFileRequest }) =>
      copyFile(workspaceId, request),
    onSuccess: (_, { workspaceId }) => {
      queryClient.invalidateQueries({
        queryKey: fileKeys.all(org!, workspaceId),
      });
    },
  });
}

/** Search files using grep */
export function useGrepFiles() {
  return useMutation({
    mutationFn: ({ workspaceId, request }: { workspaceId: string; request: GrepRequest }) =>
      grepFiles(workspaceId, request),
  });
}
