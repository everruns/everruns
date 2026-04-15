"use client";

// Organization context and provider for multitenancy
// Decision: Current org stored in localStorage for persistence across sessions
// Decision: Default org ID used as fallback
// Decision: Org selection via server-side cookie (everruns_org) for SSE compatibility
// Decision: /v1/auth/me sets org cookie if missing, so we always have org context
// Decision: On org switch, redirect entity detail pages to their list page to avoid 404s

import {
  createContext,
  useContext,
  useState,
  useEffect,
  useMemo,
  useCallback,
  useRef,
  type ReactNode,
} from "react";
import { usePathname, useRouter } from "next/navigation";
import { useQueryClient } from "@tanstack/react-query";
import { useAuth } from "./auth-provider";
import { switchOrg as switchOrgApi } from "@/lib/api/users";
import type { OrganizationMembership, OrgRole } from "@/lib/api/types";

// Default organization public ID (matches backend DEFAULT_ORG_PUBLIC_ID)
const DEFAULT_ORG_PUBLIC_ID = "org_00000000000000000000000000000001";
import { ORG_STORAGE_KEY } from "@/lib/constants";

/** Check if a role has permission for a required role level */
const ROLE_HIERARCHY: Record<OrgRole, number> = { owner: 3, admin: 2, member: 1 };

export function hasPermission(currentRole: OrgRole, requiredRole: OrgRole): boolean {
  return ROLE_HIERARCHY[currentRole] >= ROLE_HIERARCHY[requiredRole];
}

// Entity detail route prefixes — when on a detail page under one of these,
// org switch redirects to the list page to avoid a 404.
const ENTITY_PREFIXES = [
  "/agents/",
  "/apps/",
  "/capabilities/",
  "/harnesses/",
  "/sessions/",
  "/durable/schedules/",
  "/durable/workflows/",
];

/** If pathname is an entity detail page, return the parent list path. */
function getEntityListPath(pathname: string): string | null {
  for (const prefix of ENTITY_PREFIXES) {
    if (pathname.startsWith(prefix)) {
      // Strip trailing slash from prefix to get list path
      return prefix.slice(0, -1);
    }
  }
  return null;
}

export interface OrgContextValue {
  /** Currently selected organization */
  currentOrg: OrganizationMembership | null;
  /** All organizations the user belongs to */
  organizations: OrganizationMembership[];
  /** Set the current organization (awaits server cookie sync before committing) */
  setCurrentOrg: (org: OrganizationMembership) => void;
  /** Check if current user has at least the given role in the current org */
  hasRole: (role: OrgRole) => boolean;
  /** Loading state */
  isLoading: boolean;
  /** True while an org switch is in progress (cookie sync pending) */
  isSwitching: boolean;
}

export const OrgContext = createContext<OrgContextValue | undefined>(undefined);

interface OrgProviderProps {
  children: ReactNode;
  initialOrgId?: string | null;
}

export function OrgProvider({ children, initialOrgId = null }: OrgProviderProps) {
  const { user, isLoading: authLoading } = useAuth();
  const [currentOrg, setCurrentOrgState] = useState<OrganizationMembership | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [isSwitching, setIsSwitching] = useState(false);
  // Track explicit org selection to prevent useEffect from resetting it
  // before the organizations list catches up (e.g. after creating a new org).
  const explicitOrgRef = useRef<string | null>(null);
  const router = useRouter();
  const pathname = usePathname();
  const queryClient = useQueryClient();

  // Memoize organizations to prevent infinite re-renders
  // User is always fetched (even in none mode) and includes organizations
  const organizations = useMemo(() => user?.organizations ?? [], [user?.organizations]);

  // Initialize current org from localStorage or default
  useEffect(() => {
    if (authLoading || isInitialized) return;

    const storedOrgId = localStorage.getItem(ORG_STORAGE_KEY);
    const preferredOrgId = initialOrgId ?? storedOrgId;

    if (preferredOrgId && organizations.length > 0) {
      const preferredOrg = organizations.find((org) => org.public_id === preferredOrgId);
      if (preferredOrg) {
        setCurrentOrgState(preferredOrg);
        setIsInitialized(true);
        return;
      }
    }

    // Fall back to default org or first available org
    if (organizations.length > 0) {
      const defaultOrg = organizations.find((org) => org.public_id === DEFAULT_ORG_PUBLIC_ID);
      setCurrentOrgState(defaultOrg ?? organizations[0]);
    }

    setIsInitialized(true);
  }, [authLoading, organizations, isInitialized, initialOrgId]);

  // Update current org when organizations change (e.g., after login)
  useEffect(() => {
    if (!isInitialized || organizations.length === 0) return;

    // If current org is null or no longer valid, set to default
    if (!currentOrg || !organizations.find((org) => org.public_id === currentOrg.public_id)) {
      // If the user explicitly selected this org (e.g. just created it),
      // skip the reset — the organizations list hasn't caught up yet.
      if (currentOrg && explicitOrgRef.current === currentOrg.public_id) {
        return;
      }
      const defaultOrg = organizations.find((org) => org.public_id === DEFAULT_ORG_PUBLIC_ID);
      setCurrentOrgState(defaultOrg ?? organizations[0]);
    } else {
      // Org found in list — clear the explicit flag since we're in sync.
      if (explicitOrgRef.current === currentOrg?.public_id) {
        explicitOrgRef.current = null;
      }
    }
  }, [organizations, currentOrg, isInitialized]);

  // Sync currentOrg cookie on init only (fire-and-forget, not user-initiated).
  // Guard with ref to avoid redundant calls after user-initiated switches.
  const didInitSyncRef = useRef(false);
  useEffect(() => {
    if (currentOrg && isInitialized && !didInitSyncRef.current) {
      didInitSyncRef.current = true;
      if (initialOrgId === currentOrg.public_id) {
        return;
      }
      void switchOrgApi(currentOrg.public_id).catch((error) => {
        console.warn("Failed to sync org cookie on init:", error);
      });
    }
  }, [currentOrg, isInitialized, initialOrgId]);

  // Track latest switch to handle concurrent calls (last one wins)
  const switchCounterRef = useRef(0);

  // Atomic org switch: await cookie sync → commit state → invalidate queries
  const setCurrentOrg = useCallback(
    (org: OrganizationMembership) => {
      // Skip if already on this org
      if (currentOrg?.public_id === org.public_id) return;

      // Mark as explicitly selected so the sync effect doesn't reset it
      // before the organizations list is refreshed.
      explicitOrgRef.current = org.public_id;

      const switchId = ++switchCounterRef.current;
      setIsSwitching(true);

      // Await server cookie before committing client state
      switchOrgApi(org.public_id)
        .then(() => {
          // Only commit if this is still the latest switch request
          if (switchCounterRef.current !== switchId) return;
          setCurrentOrgState(org);
          localStorage.setItem(ORG_STORAGE_KEY, org.public_id);
          // Invalidate all org-scoped queries so stale data is refetched
          void queryClient.invalidateQueries();
          // Redirect entity detail pages to their list page to avoid 404
          const listPath = getEntityListPath(pathname);
          if (listPath) {
            router.push(listPath);
          }
        })
        .catch((error) => {
          // Only rollback if this is still the latest switch request
          if (switchCounterRef.current !== switchId) return;
          // Rollback: clear explicit flag, stay on current org
          explicitOrgRef.current = null;
          console.error("Org switch failed, staying on current org:", error);
        })
        .finally(() => {
          if (switchCounterRef.current === switchId) {
            setIsSwitching(false);
          }
        });
    },
    [currentOrg?.public_id, queryClient, pathname, router],
  );

  const hasRole = useCallback(
    (role: OrgRole) => {
      if (!currentOrg) return false;
      return hasPermission(currentOrg.role, role);
    },
    [currentOrg],
  );

  const value: OrgContextValue = {
    currentOrg,
    organizations,
    setCurrentOrg,
    hasRole,
    isLoading: authLoading || !isInitialized,
    isSwitching,
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
