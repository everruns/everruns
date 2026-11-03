"use client";
import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { useRouter, usePathname } from "next/navigation";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmationProvider } from "@/components/ui/confirmation-provider";
import { ThemeContext, useTheme } from "@/providers/theme-provider";
import { AuthContext, type AuthContextValue } from "@/providers/auth-provider";
import { OrgContext } from "@/providers/org-provider";
import { FeatureFlagsContext } from "@/providers/feature-flags-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { setActiveOrgId } from "@/lib/api/active-org";
import { ApiAssetContext } from "@/hooks/use-api-asset";
import type { FeatureFlags, OrganizationMembership, UserInfoResponse } from "@/lib/api/types";
import {
  connectWorkspace,
  installWorkspaceTransport,
  workspaceRequest,
  workspacePath,
  type WorkspaceInit,
} from "./transport";

interface Bootstrap {
  user: Omit<UserInfoResponse, "email_verified">;
  organization: OrganizationMembership;
  feature_flags: FeatureFlags;
  read_only: boolean;
}
const WorkspaceContext = createContext<WorkspaceInit | null>(null);
export const useMcpWorkspace = () => useContext(WorkspaceContext);

export function McpAppProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const router = useRouter();
  const pathname = usePathname();
  const ui = useUiTranslations();
  const theme = useTheme();
  const [hostTheme, setHostTheme] = useState<"light" | "dark">();
  const [init, setInit] = useState<WorkspaceInit | null>(null);
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string>();
  useEffect(() => {
    const restore = installWorkspaceTransport();
    let generation = 0;
    const disconnect = connectWorkspace((value) => {
      const current = ++generation;
      // Entry navigation belongs to the persistent provider. Catch-all page
      // instances remount on tab changes and must not reopen the initial view.
      router.replace(workspacePath(value.view));
      setInit(value);
      setError(undefined);
      setBootstrap(null);
      queryClient.clear();
      workspaceRequest<Bootstrap>("app_bootstrap")
        .then((data) => {
          if (current !== generation) return;
          setActiveOrgId(data.organization.public_id);
          setBootstrap(data);
        })
        .catch((error: Error) => {
          if (current === generation) setError(error.message);
        });
    }, setHostTheme);
    return () => {
      generation += 1;
      disconnect();
      restore();
      setActiveOrgId(null);
      queryClient.clear();
    };
  }, [queryClient, router]);
  useEffect(() => {
    window.parent.postMessage({ type: "everruns:labels", expand: ui("Expand") }, "*");
  }, [ui]);
  if (error)
    return (
      <p role="alert" className="p-6 text-destructive">
        {error}
      </p>
    );
  if (!init || !bootstrap)
    return <p className="p-6 text-sm text-muted-foreground">{ui("Loading page")}</p>;
  const org = bootstrap.organization;
  const auth: AuthContextValue = {
    config: {
      mode: "external",
      password_auth_enabled: false,
      oauth_providers: [],
      signup_enabled: false,
    },
    configLoading: false,
    configError: null,
    user: { ...bootstrap.user, email_verified: true, organizations: [org] },
    userLoading: false,
    userError: null,
    isAuthenticated: true,
    requiresAuth: true,
    isLoading: false,
    authUnavailable: false,
    logout: async () => {
      throw new Error("Manage your connection in the MCP host");
    },
    logoutPending: false,
  };
  return (
    <ThemeContext
      value={{ mode: hostTheme ?? theme.mode, theme: hostTheme ?? theme.theme, setMode: () => {} }}
    >
      <WorkspaceContext value={init}>
        <AuthContext value={auth}>
          <OrgContext
            value={{
              currentOrg: org,
              organizations: [org],
              setCurrentOrg: () => {
                throw new Error("Choose an organization when opening the MCP view");
              },
              hasRole: (role) =>
                ({ member: 1, admin: 2, owner: 3 })[org.role] >=
                { member: 1, admin: 2, owner: 3 }[role],
              isLoading: false,
              isSwitching: false,
            }}
          >
            <FeatureFlagsContext value={{ flags: bootstrap.feature_flags, isLoading: false }}>
              <ConfirmationProvider scope={`${org.public_id}:${pathname}`}>
                <ApiAssetContext value>{children}</ApiAssetContext>
              </ConfirmationProvider>
            </FeatureFlagsContext>
          </OrgContext>
        </AuthContext>
      </WorkspaceContext>
    </ThemeContext>
  );
}
