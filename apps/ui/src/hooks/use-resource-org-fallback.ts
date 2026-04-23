// Cross-org resource fallback.
// See specs/multitenancy.md (Cross-Org Resource Resolution).
//
// When a user follows a direct link to a top-level entity (session, agent,
// app, ...) that lives in an org they are a member of but have not currently
// selected, the entity API returns 404. This hook detects that condition,
// asks the backend which of the caller's orgs owns the id, and triggers an
// org switch that stays on the current page so the detail view re-fetches
// against the right org instead of redirecting to the list.

"use client";

import { useEffect, useRef } from "react";
import { ApiError } from "@/lib/api/client";
import { resolveOrgForResource } from "@/lib/api/resolver";
import { useOrg } from "@/providers/org-provider";

interface Options {
  /** Prefixed public id from the current route (e.g. `session_019db...`). */
  resourceId: string | undefined;
  /**
   * The resource query's error (React Query's `error` field). The hook only
   * reacts when it looks like a genuine 404 against the current org.
   */
  error: unknown;
  /** True while the resource query is in flight. */
  isLoading: boolean;
}

function is404(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404;
}

/**
 * Attempt to recover from a 404 on an entity detail route by switching to the
 * owning org (if the caller is a member of it).
 *
 * Calling sites keep their existing "not found" fallback UI — if the resolver
 * can't recover, nothing changes. If it can, `setCurrentOrg` is invoked with
 * `stayOnPage: true`, React Query invalidates, and the detail query re-runs
 * in the new org context.
 */
export function useResourceOrgFallback({ resourceId, error, isLoading }: Options): void {
  const { currentOrg, organizations, setCurrentOrg, isSwitching } = useOrg();
  const attemptedRef = useRef<string | null>(null);

  useEffect(() => {
    if (!resourceId || !currentOrg || isLoading || isSwitching) return;
    if (!is404(error)) return;
    // Only try once per resource id per mount to avoid ping-pong if the
    // target org also returns 404.
    if (attemptedRef.current === resourceId) return;
    attemptedRef.current = resourceId;

    let cancelled = false;
    (async () => {
      try {
        const result = await resolveOrgForResource(resourceId);
        if (cancelled || !result) return;
        if (result.org_id === currentOrg.public_id) return;
        const target = organizations.find((o) => o.public_id === result.org_id);
        if (!target) return;
        setCurrentOrg(target, { stayOnPage: true });
      } catch (e) {
        console.warn("Failed to resolve owning org for resource:", e);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [resourceId, error, isLoading, isSwitching, currentOrg, organizations, setCurrentOrg]);
}
