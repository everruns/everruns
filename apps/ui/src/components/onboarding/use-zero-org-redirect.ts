"use client";

import { useEffect } from "react";
import { usePathname, useRouter } from "next/navigation";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";

/** Route for the OSS-owned zero-org onboarding surface. */
export const ONBOARDING_PATH = "/onboarding";

/**
 * Redirects authenticated users with zero organizations to the onboarding
 * surface, returning `true` while the redirect is pending so callers can avoid
 * flashing the app shell.
 *
 * Decision: exported so SaaS-style wrappers reuse the OSS redirect from their
 * own main layout instead of duplicating it. No-op while auth/org state is
 * loading, when auth is not required (OSS seeds a default org so there is never
 * a zero-org state), once the user belongs to at least one org, or while
 * already on the onboarding route.
 */
export function useZeroOrgRedirect(): boolean {
  const router = useRouter();
  const pathname = usePathname();
  const { isAuthenticated, requiresAuth, isLoading: authLoading } = useAuth();
  const { organizations, isLoading: orgLoading } = useOrg();

  const shouldRedirect =
    !authLoading &&
    !orgLoading &&
    requiresAuth &&
    isAuthenticated &&
    organizations.length === 0 &&
    pathname !== ONBOARDING_PATH;

  useEffect(() => {
    if (shouldRedirect) {
      router.replace(ONBOARDING_PATH);
    }
  }, [shouldRedirect, router]);

  return shouldRedirect;
}
