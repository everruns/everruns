// Cross-org resource fallback.
// See knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
//
// When a user follows a direct link to a top-level entity (session, agent,
// app, ...) that lives in an org they are a member of but have not currently
// selected, the entity API returns 404. This hook detects that condition,
// asks the backend which of the caller's orgs owns the id, and triggers an
// org switch that stays on the current page so the detail view re-fetches
// against the right org instead of redirecting to the list.

"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

export interface ResourceOrgFallbackState {
  /**
   * True while a 404 might still be recoverable by switching to another org.
   * Detail pages should hide final "not found" UI while this is true.
   */
  isCheckingOtherOrgs: boolean;
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
export function useResourceOrgFallback({
  resourceId,
  error,
  isLoading,
}: Options): ResourceOrgFallbackState {
  const { currentOrg, organizations = [], setCurrentOrg, isSwitching } = useOrg();
  const attemptedKeysRef = useRef<Set<string>>(new Set());
  const [exhaustedKeys, setExhaustedKeys] = useState<Set<string>>(() => new Set());

  const lookupKey = resourceId && currentOrg ? `${resourceId}:${currentOrg.public_id}` : undefined;
  const shouldCheck =
    !!lookupKey && !isLoading && !isSwitching && is404(error) && !exhaustedKeys.has(lookupKey);

  const organizationsById = useMemo(
    () => new Map(organizations.map((org) => [org.public_id, org])),
    [organizations],
  );

  const markExhausted = useCallback((key: string) => {
    setExhaustedKeys((prev) => {
      if (prev.has(key)) return prev;
      const next = new Set(prev);
      next.add(key);
      return next;
    });
  }, []);

  useEffect(() => {
    if (!resourceId || !currentOrg || !lookupKey || !shouldCheck) return;
    // Only try once per resource+org pair. If switching succeeds but the
    // target org still 404s, the key changes and we can mark that final state
    // exhausted instead of hiding the error forever.
    if (attemptedKeysRef.current.has(lookupKey)) return;
    attemptedKeysRef.current.add(lookupKey);

    let cancelled = false;
    (async () => {
      try {
        const result = await resolveOrgForResource(resourceId);
        if (cancelled) return;
        if (!result || result.org_id === currentOrg.public_id) {
          markExhausted(lookupKey);
          return;
        }
        const target = organizationsById.get(result.org_id);
        if (!target) {
          markExhausted(lookupKey);
          return;
        }
        setCurrentOrg(target, { stayOnPage: true });
        markExhausted(lookupKey);
      } catch (e) {
        if (!cancelled) {
          markExhausted(lookupKey);
        }
        console.warn("Failed to resolve owning org for resource:", e);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    resourceId,
    currentOrg,
    lookupKey,
    shouldCheck,
    organizationsById,
    markExhausted,
    setCurrentOrg,
  ]);

  return { isCheckingOtherOrgs: shouldCheck || (isSwitching && is404(error)) };
}
