"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { getOrganization } from "@/lib/api/organizations";
import { queryKeys } from "@/lib/query-keys";
import { ONBOARDING_PATH } from "./use-zero-org-redirect";

/** Matches the org setup wizard route: `/orgs/{orgId}/setup`. */
const SETUP_ROUTE_RE = /^\/orgs\/[^/]+\/setup$/;

/**
 * True for any route that IS the onboarding surface (the zero-org page or an
 * org's setup wizard). Used to (a) suppress the resume redirect while already
 * there and (b) let the main layout render these routes full-screen without the
 * app sidebar.
 */
export function isOnboardingRoute(pathname: string | null): boolean {
  if (!pathname) return false;
  return pathname === ONBOARDING_PATH || SETUP_ROUTE_RE.test(pathname);
}

/**
 * Resumes an interrupted onboarding: if the CURRENT org exists but its
 * `onboarding_completed_at` is still null, redirect the user back to
 * `/orgs/{currentOrgId}/setup`. Returns `true` while a redirect is pending so
 * the caller can avoid flashing the app shell.
 *
 * Existing orgs are unaffected: the 090 migration backfills every pre-existing
 * org (and seeded/external orgs are created already-complete), so only a
 * brand-new org that is mid-onboarding has a null marker.
 *
 * Loop/flash guards:
 * - never redirects while auth/org/org-detail data is still loading;
 * - never redirects when already on an onboarding/setup route;
 * - only redirects on an explicit `onboarding_completed_at === null` (a load
 *   error or missing org is treated as "don't redirect", failing open into the
 *   normal app rather than trapping the user).
 */
export function useOnboardingResumeRedirect(): boolean {
  const router = useRouter();
  const pathname = usePathname();
  const { isAuthenticated, requiresAuth, isLoading: authLoading } = useAuth();
  const { currentOrg, isLoading: orgLoading } = useOrg();

  const currentOrgId = currentOrg?.public_id ?? null;

  // Already on an onboarding/setup route — nothing to resume, and querying here
  // would risk a redirect loop.
  const onOnboardingRoute = isOnboardingRoute(pathname);

  const canQuery =
    !authLoading &&
    !orgLoading &&
    requiresAuth &&
    isAuthenticated &&
    !!currentOrgId &&
    !onOnboardingRoute;

  const { data: org, isLoading: orgDetailLoading } = useQuery({
    queryKey: queryKeys.organizations.detail(currentOrgId ?? ""),
    queryFn: () => getOrganization(currentOrgId as string),
    enabled: canQuery,
    staleTime: 30_000,
  });

  const shouldRedirect =
    canQuery && !orgDetailLoading && !!org && org.onboarding_completed_at == null;

  useEffect(() => {
    if (shouldRedirect && currentOrgId) {
      router.replace(`/orgs/${currentOrgId}/setup`);
    }
  }, [shouldRedirect, currentOrgId, router]);

  return shouldRedirect;
}
