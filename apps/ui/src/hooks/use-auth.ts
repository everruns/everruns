// Auth hooks using React Query
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getAuthConfig,
  login,
  register,
  getCurrentUser,
  logout,
  listPersonalAccessTokens,
  createPersonalAccessToken,
  deletePersonalAccessToken,
  forgotPassword,
  resetPassword,
  verifyEmail,
  resendVerification,
} from "@/lib/api/auth";
import { updateProfile } from "@/lib/api/users";
import { useOrg } from "@/providers/org-provider";
import { ORG_STORAGE_KEY } from "@/lib/constants";
import { authQueryKeys } from "@/lib/auth-query-keys";
import type {
  LoginRequest,
  RegisterRequest,
  CreatePersonalAccessTokenRequest,
  UpdateProfileRequest,
  ForgotPasswordRequest,
  ResetPasswordRequest,
  VerifyEmailRequest,
  ResendVerificationRequest,
} from "@/lib/api/types";

export const authKeys = authQueryKeys;

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
 * Hook for the forgot-password mutation (enumeration-safe).
 */
export function useForgotPassword() {
  return useMutation({
    mutationFn: (request: ForgotPasswordRequest) => forgotPassword(request),
  });
}

/**
 * Hook for the reset-password mutation.
 */
export function useResetPassword() {
  return useMutation({
    mutationFn: (request: ResetPasswordRequest) => resetPassword(request),
  });
}

/**
 * Hook for the verify-email mutation.
 */
export function useVerifyEmail() {
  return useMutation({
    mutationFn: (request: VerifyEmailRequest) => verifyEmail(request),
  });
}

/**
 * Hook for the resend-verification mutation (enumeration-safe).
 */
export function useResendVerification() {
  return useMutation({
    mutationFn: (request: ResendVerificationRequest) => resendVerification(request),
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
      // Clear ALL cached queries to prevent data leaking across users.
      // Selective removal (only auth keys) left org-sensitive data
      // (durable, admin, sessions, etc.) cached for the next user.
      queryClient.clear();
      // Clear persisted org selection
      localStorage.removeItem(ORG_STORAGE_KEY);
    },
  });
}

/**
 * Hook to list personal access tokens (scoped per org to avoid stale cache after org switch)
 */
export function usePersonalAccessTokens() {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;

  const query = useQuery({
    queryKey: authKeys.personalAccessTokens(org),
    queryFn: listPersonalAccessTokens,
    enabled: !!org,
    retry: false,
  });

  return {
    ...query,
    isLoading: orgLoading || query.isLoading,
  };
}

/**
 * Hook to create a personal access token
 */
export function useCreatePersonalAccessToken() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (request: CreatePersonalAccessTokenRequest) => createPersonalAccessToken(request),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: authKeys.personalAccessTokens(org),
      });
    },
  });
}

/**
 * Hook to delete a personal access token
 */
export function useDeletePersonalAccessToken() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (tokenId: string) => deletePersonalAccessToken(tokenId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: authKeys.personalAccessTokens(org),
      });
    },
  });
}

/**
 * Hook to update current user's profile
 */
export function useUpdateProfile() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: UpdateProfileRequest) => updateProfile(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: authKeys.user() });
    },
  });
}
