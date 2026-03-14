"use client";

// AuthProvider - handles authentication state and conditional rendering
// Decision: Use React Context + React Query for auth state management
// Decision: Check auth config first, then only require login if auth is enabled
// Decision: logout and createOrganization are pluggable via provider props (EVE-53)
//   SaaS layers can inject overrides without re-exporting every hook from use-auth.ts

import { createContext, useContext, useCallback, type ReactNode } from "react";
import { useAuthConfig, useCurrentUser, useLogout as useLogoutMutation } from "@/hooks/use-auth";
import { ApiError } from "@/lib/api/client";
import type { AuthConfigResponse, UserInfoResponse } from "@/lib/api/types";

export interface AuthContextValue {
  // Auth configuration from server
  config: AuthConfigResponse | undefined;
  configLoading: boolean;
  configError: Error | null;

  // Current user (null if not authenticated, undefined if loading)
  user: UserInfoResponse | null | undefined;
  userLoading: boolean;
  userError: Error | null;

  // Derived state
  isAuthenticated: boolean;
  requiresAuth: boolean;
  isLoading: boolean;
  authUnavailable: boolean;

  // Pluggable actions — defaults call OSS endpoints; SaaS can override at provider level
  logout: () => Promise<void>;
  logoutPending: boolean;
  createOrganization?: () => void;
}

export const AuthContext = createContext<AuthContextValue | undefined>(undefined);

function isUnauthenticatedError(error: Error | null): boolean {
  return error instanceof ApiError && error.status === 401;
}

interface AuthProviderProps {
  children: ReactNode;
  /** Override default logout (POST /v1/auth/logout + cache clear). */
  logout?: () => Promise<void>;
  /** Override default create-organization action (shows built-in dialog by default). */
  createOrganization?: () => void;
}

export function AuthProvider({
  children,
  logout: logoutOverride,
  createOrganization,
}: AuthProviderProps) {
  const { data: config, isLoading: configLoading, error: configError } = useAuthConfig();

  // Always fetch user - this also sets the org cookie if missing
  // In "none" mode, returns anonymous user with default org
  const { data: user, isLoading: userLoading, error: userError } = useCurrentUser(!!config);

  // Default logout via React Query mutation
  const logoutMutation = useLogoutMutation();
  const defaultLogout = useCallback(async () => {
    await logoutMutation.mutateAsync();
  }, [logoutMutation]);

  // Determine if authentication is required based on mode
  const requiresAuth = config ? config.mode !== "none" : false;
  const authUnavailable = !!configError || (!!userError && !isUnauthenticatedError(userError));

  // User is authenticated if:
  // 1. Auth is not required (mode=none), OR
  // 2. User data exists
  const isAuthenticated = !authUnavailable && (!requiresAuth || !!user);

  // Overall loading state - always wait for user fetch (sets org cookie)
  const isLoading = configLoading || userLoading;

  const value: AuthContextValue = {
    config,
    configLoading,
    configError: configError as Error | null,
    user: user ?? null,
    userLoading,
    userError: userError as Error | null,
    isAuthenticated,
    requiresAuth,
    isLoading,
    authUnavailable,
    logout: logoutOverride ?? defaultLogout,
    logoutPending: logoutOverride ? false : logoutMutation.isPending,
    createOrganization,
  };

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (context === undefined) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return context;
}
