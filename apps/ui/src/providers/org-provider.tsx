"use client";

// Organization context and provider for multitenancy
// Decision: Current org stored in localStorage for persistence across sessions
// Decision: Default org ID used as fallback
// Decision: When AUTH_MODE=none, use default org directly
// Decision: Org selection via server-side cookie (everruns_org) for SSE compatibility

import {
  createContext,
  useContext,
  useState,
  useEffect,
  useMemo,
  useCallback,
  type ReactNode,
} from "react";
import { useAuth } from "./auth-provider";
import { switchOrg as switchOrgApi } from "@/lib/api/users";
import type { OrganizationMembership } from "@/lib/api/types";

// Default organization public ID (matches backend DEFAULT_ORG_PUBLIC_ID)
const DEFAULT_ORG_PUBLIC_ID = "org_00000000000000000000000000000001";
const STORAGE_KEY = "everruns_current_org";

// Default organization membership for anonymous auth mode
const DEFAULT_ORG_MEMBERSHIP: OrganizationMembership = {
  public_id: DEFAULT_ORG_PUBLIC_ID,
  name: "Default Organization",
};

interface OrgContextValue {
  /** Currently selected organization */
  currentOrg: OrganizationMembership | null;
  /** All organizations the user belongs to */
  organizations: OrganizationMembership[];
  /** Set the current organization */
  setCurrentOrg: (org: OrganizationMembership) => void;
  /** Loading state */
  isLoading: boolean;
}

const OrgContext = createContext<OrgContextValue | undefined>(undefined);

interface OrgProviderProps {
  children: ReactNode;
}

export function OrgProvider({ children }: OrgProviderProps) {
  const { user, isLoading: authLoading, requiresAuth } = useAuth();
  const [currentOrg, setCurrentOrgState] = useState<OrganizationMembership | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);

  // Memoize organizations to prevent infinite re-renders
  // When auth is not required (AUTH_MODE=none), use default org
  const organizations = useMemo(
    () => {
      if (!requiresAuth) {
        // Anonymous mode: use default organization
        return [DEFAULT_ORG_MEMBERSHIP];
      }
      return user?.organizations ?? [];
    },
    [user?.organizations, requiresAuth]
  );

  // Initialize current org from localStorage or default
  useEffect(() => {
    if (authLoading || isInitialized) return;

    // Try to restore from localStorage
    const storedOrgId = localStorage.getItem(STORAGE_KEY);

    if (storedOrgId && organizations.length > 0) {
      // Find the stored org in user's organizations
      const storedOrg = organizations.find(org => org.public_id === storedOrgId);
      if (storedOrg) {
        setCurrentOrgState(storedOrg);
        setIsInitialized(true);
        return;
      }
    }

    // Fall back to default org or first available org
    if (organizations.length > 0) {
      const defaultOrg = organizations.find(org => org.public_id === DEFAULT_ORG_PUBLIC_ID);
      setCurrentOrgState(defaultOrg ?? organizations[0]);
    }

    setIsInitialized(true);
  }, [authLoading, organizations, isInitialized]);

  // Update current org when organizations change (e.g., after login)
  useEffect(() => {
    if (!isInitialized || organizations.length === 0) return;

    // If current org is no longer valid, reset to default
    if (currentOrg && !organizations.find(org => org.public_id === currentOrg.public_id)) {
      const defaultOrg = organizations.find(org => org.public_id === DEFAULT_ORG_PUBLIC_ID);
      setCurrentOrgState(defaultOrg ?? organizations[0]);
    }
  }, [organizations, currentOrg, isInitialized]);

  // Call API to switch org (sets server-side cookie)
  const syncOrgCookie = useCallback(async (orgId: string) => {
    try {
      await switchOrgApi(orgId);
    } catch (error) {
      // Log but don't throw - org switching should be resilient
      console.warn("Failed to sync org cookie:", error);
    }
  }, []);

  const setCurrentOrg = useCallback((org: OrganizationMembership) => {
    setCurrentOrgState(org);
    localStorage.setItem(STORAGE_KEY, org.public_id);
    // Set server-side cookie via API
    void syncOrgCookie(org.public_id);
  }, [syncOrgCookie]);

  // Sync currentOrg to server cookie on mount/change
  useEffect(() => {
    if (currentOrg && isInitialized) {
      void syncOrgCookie(currentOrg.public_id);
    }
  }, [currentOrg, isInitialized, syncOrgCookie]);

  const value: OrgContextValue = {
    currentOrg,
    organizations,
    setCurrentOrg,
    isLoading: authLoading || !isInitialized,
  };

  return <OrgContext.Provider value={value}>{children}</OrgContext.Provider>;
}

export function useOrg() {
  const context = useContext(OrgContext);
  if (context === undefined) {
    throw new Error("useOrg must be used within an OrgProvider");
  }
  return context;
}
