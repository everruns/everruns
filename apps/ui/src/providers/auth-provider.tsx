"use client";

// AuthProvider - handles authentication state and conditional rendering
// Decision: Use React Context + React Query for auth state management
// Decision: Check auth config first, then only require login if auth is enabled

import { createContext, useContext, type ReactNode } from "react";
import { useAuthConfig, useCurrentUser } from "@/hooks/use-auth";
import type { AuthConfigResponse, UserInfoResponse } from "@/lib/api/types";

interface AuthContextValue {
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
}

const AuthContext = createContext<AuthContextValue | undefined>(undefined);

interface AuthProviderProps {
  children: ReactNode;
}

export function AuthProvider({ children }: AuthProviderProps) {
  const {
    data: config,
    isLoading: configLoading,
    error: configError,
  } = useAuthConfig();

  // Always fetch user - this also sets the org cookie if missing
  // In "none" mode, returns anonymous user with default org
  const {
    data: user,
    isLoading: userLoading,
    error: userError,
  } = useCurrentUser(!!config);

  // Determine if authentication is required based on mode
  const requiresAuth = config ? config.mode !== "none" : false;

  // User is authenticated if:
  // 1. Auth is not required (mode=none), OR
  // 2. User data exists
  const isAuthenticated = !requiresAuth || !!user;

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
