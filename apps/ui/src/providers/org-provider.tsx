"use client";

// Organization context and provider for multitenancy
// Decision: Current org stored in localStorage for persistence across sessions
// Decision: Default org ID used as fallback
// Decision: Org selection via server-side cookie (everruns_org) for SSE compatibility
// Decision: /v1/platform/auth/me sets org cookie if missing, so we always have org context
// Decision: On org switch, redirect entity detail pages to their list page to avoid 404s
// Decision: Every org commit publishes the org to the API layer (X-Org-Id) synchronously,
//   because the cookie sync is a separate request that lands later — requests issued in
//   between were answered in the previous org and cached under the new one, which is how
//   a second Platform Chat thread got created on a page that already had one.
// Decision: A page load that opens an entity link (e.g. /sessions/session_…) resolves
//   the owning org *before* the first org commit. Starting in the stored org and
//   switching after the entity 404s loaded the whole app twice (wrong org, then
//   owning org) and tripped the edge rate limit. Resolution is bounded by a
//   timeout; on any failure the stored org is used and the in-page fallback
//   (hooks/use-resource-org-fallback.ts) still recovers.
// Decision: Every switch the app makes on the user's behalf to open a page is
//   published as `autoSwitch`, and the shell announces it (OrgSwitchNotice) with a
//   way back. A silent switch made another org's data look like a cross-tenant leak.

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
import { resolveOrgForResource } from "@/lib/api/resolver";
import { getActiveOrgId, setActiveOrgId } from "@/lib/api/active-org";
import type { OrganizationMembership, OrgRole } from "@/lib/api/types";

// Default organization public ID (matches backend DEFAULT_ORG_PUBLIC_ID)
const DEFAULT_ORG_PUBLIC_ID = "org_00000000000000000000000000000001";
import { ORG_STORAGE_KEY } from "@/lib/constants";

/** Check if a role has permission for a required role level */
const ROLE_HIERARCHY: Record<OrgRole, number> = {
  owner: 3,
  admin: 2,
  member: 1,
};

export function hasPermission(currentRole: OrgRole, requiredRole: OrgRole): boolean {
  return ROLE_HIERARCHY[currentRole] >= ROLE_HIERARCHY[requiredRole];
}

// Entity detail route prefixes — when on a detail page under one of these,
// org switch redirects to the list page to avoid a 404. MUST stay in sync
// with the dynamic [id] routes under apps/ui/src/app/(main)/.
const ENTITY_PREFIXES = [
  "/playground/",
  "/virtual-users/",
  "/agents/",
  "/capabilities/",
  "/evals/",
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

/** Prefixed public id (`session_<32 hex>`) on an entity detail route, if any. */
export function entityIdFromPath(pathname: string | null | undefined): string | null {
  if (!pathname) return null;
  for (const prefix of ENTITY_PREFIXES) {
    if (!pathname.startsWith(prefix)) continue;
    const segment = pathname.slice(prefix.length).split("/", 1)[0];
    return /^[a-z][a-z_]*_[0-9a-f]{32}$/.test(segment) ? segment : null;
  }
  return null;
}

/** How long a landing waits for the owning org before using the stored one. */
const LANDING_RESOLVE_TIMEOUT_MS = 3000;

/** The owning org of the entity a page load opened, or null when unknown. */
async function resolveLandingOrg(entityId: string): Promise<string | null> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<null>((resolve) => {
    timer = setTimeout(() => resolve(null), LANDING_RESOLVE_TIMEOUT_MS);
  });
  try {
    const result = await Promise.race([resolveOrgForResource(entityId).catch(() => null), timeout]);
    return result?.org_id ?? null;
  } finally {
    clearTimeout(timer);
  }
}

export interface SetCurrentOrgOptions {
  /**
   * When true, skip the default "redirect entity detail to list" behaviour and
   * stay on the current page after the switch. Used by the cross-org resource
   * resolver — after switching to the owning org, the current detail URL is
   * valid and should not be redirected away from.
   */
  stayOnPage?: boolean;
}

/** A switch the app made by itself to open a page in the org that owns it. */
export interface AutoOrgSwitch {
  /** Increments per switch, so each one is announced once. */
  id: number;
  /** The org the user was in, when it is still one of theirs. */
  from: OrganizationMembership | null;
  to: OrganizationMembership;
}

export interface OrgContextValue {
  /** Currently selected organization */
  currentOrg: OrganizationMembership | null;
  /** All organizations the user belongs to */
  organizations: OrganizationMembership[];
  /** Set the current organization (awaits server cookie sync before committing) */
  setCurrentOrg: (org: OrganizationMembership, options?: SetCurrentOrgOptions) => void;
  /** Check if current user has at least the given role in the current org */
  hasRole: (role: OrgRole) => boolean;
  /** Loading state */
  isLoading: boolean;
  /** True while an org switch is in progress (cookie sync pending) */
  isSwitching: boolean;
  /** The last switch the app made by itself to open a page, if any. */
  autoSwitch?: AutoOrgSwitch | null;
}

export const OrgContext = createContext<OrgContextValue | undefined>(undefined);

interface OrgProviderProps {
  children: ReactNode;
  initialOrgId?: string | null;
}

export function OrgProvider({ children, initialOrgId = null }: OrgProviderProps) {
  const { user, isLoading: authLoading } = useAuth();
  // Published during render, not from an effect: children hydrate their
  // server-seeded query entries while rendering, and those entries were hashed
  // on the server under this same cookie org (see lib/server-query.ts). An
  // effect runs after that, too late to match. Idempotent, so a double render
  // costs nothing.
  if (initialOrgId && getActiveOrgId() === null) {
    setActiveOrgId(initialOrgId);
  }
  const [currentOrg, setCurrentOrgStateRaw] = useState<OrganizationMembership | null>(null);
  // Publish to the API layer before the state commit, so the very first request
  // a consumer makes in the new org already carries `X-Org-Id`.
  const setCurrentOrgState = useCallback((org: OrganizationMembership | null) => {
    setActiveOrgId(org?.public_id ?? null);
    setCurrentOrgStateRaw(org);
  }, []);
  const [isInitialized, setIsInitialized] = useState(false);
  const [isSwitching, setIsSwitching] = useState(false);
  const [autoSwitch, setAutoSwitch] = useState<AutoOrgSwitch | null>(null);
  const announceSwitch = useCallback(
    (from: OrganizationMembership | null | undefined, to: OrganizationMembership) => {
      if (from?.public_id === to.public_id) return;
      setAutoSwitch((prev) => ({ id: (prev?.id ?? 0) + 1, from: from ?? null, to }));
    },
    [],
  );
  // Track explicit org selection to prevent useEffect from resetting it
  // before the organizations list catches up (e.g. after creating a new org).
  const explicitOrgRef = useRef<string | null>(null);
  const router = useRouter();
  const pathname = usePathname();
  const queryClient = useQueryClient();

  // Memoize organizations to prevent infinite re-renders
  // User is always fetched (even in none mode) and includes organizations
  const organizations = useMemo(() => user?.organizations ?? [], [user?.organizations]);

  // Set once the cookie is known to name the current org (see the init sync).
  const didInitSyncRef = useRef(false);

  // Resolve the owning org of an entity link once, at page load, in parallel
  // with auth. `undefined` = still resolving; null = nothing to resolve.
  const [landingOrgId, setLandingOrgId] = useState<string | null | undefined>(() =>
    entityIdFromPath(pathname) ? undefined : null,
  );
  const landingEntityRef = useRef(entityIdFromPath(pathname));
  useEffect(() => {
    const entityId = landingEntityRef.current;
    if (!entityId) return;
    let cancelled = false;
    void resolveLandingOrg(entityId).then((orgId) => {
      if (!cancelled) setLandingOrgId(orgId);
    });
    return () => {
      cancelled = true;
    };
  }, []);
  // True while the landing org is being made the cookie org before commit.
  const landingSyncRef = useRef(false);

  // Initialize current org from the landing entity, localStorage or default
  useEffect(() => {
    if (authLoading || isInitialized || landingSyncRef.current) return;
    // Wait for the landing resolution only when the user has orgs to pick from.
    if (landingOrgId === undefined && organizations.length > 0) return;

    const landingOrg = landingOrgId
      ? organizations.find((org) => org.public_id === landingOrgId)
      : undefined;
    if (landingOrg) {
      // The org this load would have used without the link: the one to announce
      // leaving, and to offer going back to.
      const storedOrgId = localStorage.getItem(ORG_STORAGE_KEY);
      const previousOrg = organizations.find(
        (org) => org.public_id === (initialOrgId ?? storedOrgId),
      );
      const commit = () => {
        setCurrentOrgState(landingOrg);
        announceSwitch(previousOrg, landingOrg);
        localStorage.setItem(ORG_STORAGE_KEY, landingOrg.public_id);
        // The cookie already names this org; the init sync below has nothing to do.
        didInitSyncRef.current = true;
        setIsInitialized(true);
      };
      if (landingOrg.public_id === initialOrgId) {
        commit();
        return;
      }
      // Live streams authenticate the org by cookie, so make the landing org
      // the cookie org before anything mounts in it.
      landingSyncRef.current = true;
      void switchOrgApi(landingOrg.public_id)
        .catch((error) => {
          console.warn("Failed to sync org cookie for the landing org:", error);
        })
        .finally(() => {
          landingSyncRef.current = false;
          commit();
        });
      return;
    }

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
      const nextOrg = defaultOrg ?? organizations[0];
      setCurrentOrgState(nextOrg);
      // Keep server-side org cookie in sync for non-explicit org changes
      // (e.g. membership removal, login org-list refresh).
      void switchOrgApi(nextOrg.public_id).catch((error) => {
        console.warn("Failed to sync org cookie after automatic org update:", error);
      });
    }

    setIsInitialized(true);
  }, [authLoading, organizations, isInitialized, initialOrgId, landingOrgId, announceSwitch]);

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
      const nextOrg = defaultOrg ?? organizations[0];
      setCurrentOrgState(nextOrg);
      // Keep server-side org cookie in sync for non-explicit org changes
      // (e.g. membership removal, login org-list refresh).
      void switchOrgApi(nextOrg.public_id).catch((error) => {
        console.warn("Failed to sync org cookie after automatic org update:", error);
      });
    } else {
      const refreshed = organizations.find((org) => org.public_id === currentOrg.public_id);
      // A rename or role change is metadata for this same organization. Keep
      // its mounted pages/Chat and caches, while refreshing labels and controls.
      if (refreshed && (refreshed.name !== currentOrg.name || refreshed.role !== currentOrg.role)) {
        setCurrentOrgState(refreshed);
      }
      // Org found in list — clear the explicit flag since we're in sync.
      if (explicitOrgRef.current === currentOrg?.public_id) {
        explicitOrgRef.current = null;
      }
    }
  }, [organizations, currentOrg, isInitialized]);

  // Sync currentOrg cookie on init only (fire-and-forget, not user-initiated).
  // Guard with ref to avoid redundant calls after user-initiated switches.
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

  // Atomic org switch: await cookie sync → commit state → mark queries stale
  const setCurrentOrg = useCallback(
    (org: OrganizationMembership, options?: SetCurrentOrgOptions) => {
      // Skip if already on this org
      if (currentOrg?.public_id === org.public_id) return;

      // Mark as explicitly selected so the sync effect doesn't reset it
      // before the organizations list is refreshed.
      explicitOrgRef.current = org.public_id;

      const switchId = ++switchCounterRef.current;
      setIsSwitching(true);
      const stayOnPage = options?.stayOnPage === true;
      const previousOrg = currentOrg;

      // Await server cookie before committing client state
      switchOrgApi(org.public_id)
        .then(() => {
          // Only commit if this is still the latest switch request
          if (switchCounterRef.current !== switchId) return;
          setCurrentOrgState(org);
          localStorage.setItem(ORG_STORAGE_KEY, org.public_id);
          // Mark every cached entry stale, but refetch nothing here. Cache
          // identity includes the org (lib/query-client.ts), so each mounted
          // query re-renders onto a new-org entry and fetches it once. A blanket
          // refetch at this point ran before that re-render: it re-sent every
          // active query against its previous-org entry (in the new org, by
          // header), then the re-render sent them all again. One switch cost
          // the whole app's queries twice and tripped the edge rate limit.
          void queryClient.invalidateQueries({ refetchType: "none" });
          // Redirect entity detail pages to their list page to avoid 404,
          // unless the caller has opted out (e.g. cross-org resource
          // resolver, where the detail URL is valid in the new org).
          // Staying on the page means the app switched to open it: announce it.
          if (stayOnPage) announceSwitch(previousOrg, org);
          if (!stayOnPage) {
            const listPath = getEntityListPath(pathname);
            if (listPath) {
              router.push(listPath);
            }
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
    [currentOrg, queryClient, pathname, router, announceSwitch],
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
    autoSwitch,
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
