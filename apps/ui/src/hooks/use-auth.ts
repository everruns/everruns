// Auth hooks using React Query
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getAuthConfig,
  login,
  register,
  getCurrentUser,
  logout,
  listApiKeys,
  createApiKey,
  deleteApiKey,
} from "@/lib/api/auth";
import type {
  LoginRequest,
  RegisterRequest,
  CreateApiKeyRequest,
} from "@/lib/api/types";

// Query keys
export const authKeys = {
  all: ["auth"] as const,
  config: () => [...authKeys.all, "config"] as const,
  user: () => [...authKeys.all, "user"] as const,
  apiKeys: () => [...authKeys.all, "api-keys"] as const,
};

/**
 * Hook to get auth configuration.
 * This is the primary way the UI determines what auth mode is active.
 */
export function useAuthConfig() {
  return useQuery({
    queryKey: authKeys.config(),
    queryFn: getAuthConfig,
    staleTime: 5 * 60 * 1000, // Config rarely changes, cache for 5 minutes
    retry: 1,
  });
}

/**
 * Hook to get current user info.
 * Returns null/undefined if not authenticated.
 * @param enabled - Whether to fetch user (default: true). Set to false when auth is not required.
 */
export function useCurrentUser(enabled: boolean = true) {
  return useQuery({
    queryKey: authKeys.user(),
    queryFn: getCurrentUser,
    enabled, // Only fetch when enabled
    retry: false, // Don't retry on 401
    staleTime: 60 * 1000, // Cache for 1 minute
  });
}

/**
 * Hook for login mutation
 */
export function useLogin() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: LoginRequest) => login(request),
    onSuccess: async () => {
      // Refetch user data after login and wait for it to complete
      await queryClient.refetchQueries({ queryKey: authKeys.user() });
    },
  });
}

/**
 * Hook for register mutation
 */
export function useRegister() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: RegisterRequest) => register(request),
    onSuccess: async () => {
      // Refetch user data after registration and wait for it to complete
      await queryClient.refetchQueries({ queryKey: authKeys.user() });
    },
  });
}

/**
 * Hook for logout mutation
 */
export function useLogout() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: logout,
    onSuccess: () => {
      // Clear all auth-related queries
      queryClient.removeQueries({ queryKey: authKeys.user() });
      queryClient.removeQueries({ queryKey: authKeys.apiKeys() });
    },
  });
}

/**
 * Hook to list API keys
 */
export function useApiKeys() {
  return useQuery({
    queryKey: authKeys.apiKeys(),
    queryFn: listApiKeys,
    retry: false,
  });
}

/**
 * Hook to create an API key
 */
export function useCreateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateApiKeyRequest) => createApiKey(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: authKeys.apiKeys() });
    },
  });
}

/**
 * Hook to delete an API key
 */
export function useDeleteApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (keyId: string) => deleteApiKey(keyId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: authKeys.apiKeys() });
    },
  });
}
