"use client";

// Main app layout with sidebar, auth guard, and global command palette
import { Suspense, useEffect } from "react";
import { useRouter, usePathname, useSearchParams } from "next/navigation";
import { Loader2 } from "lucide-react";
import { Sidebar } from "@/components/layout/sidebar";
import { EarlyAccessBanner } from "@/components/layout/early-access-banner";
import { VerifyEmailBanner } from "@/components/layout/verify-email-banner";
import { CommandPalette } from "@/components/command-palette";
import { AuthUnavailableState } from "@/components/layout/auth-unavailable-state";
import { CommandPaletteContext, useCommandPaletteState } from "@/hooks/use-command-palette";
import {
  getLoginRedirectPath,
  isBackendNavigationPath,
  RETURN_TO_STORAGE_KEY,
  sanitizeReturnTo,
} from "@/lib/auth-redirect";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { NotificationsProvider } from "@/providers/notifications-provider";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useZeroOrgRedirect } from "@/components/onboarding/use-zero-org-redirect";
import {
  isOnboardingRoute,
  useOnboardingResumeRedirect,
} from "@/components/onboarding/use-onboarding-resume-redirect";

interface MainLayoutProps {
  children: React.ReactNode;
}

function MainLayoutInner({ children }: MainLayoutProps) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { isAuthenticated, isLoading: authLoading, requiresAuth, authUnavailable } = useAuth();
  const { isLoading: orgLoading } = useOrg();
  const notificationsEnabled = useFeatureFlag("notifications");
  const commandPalette = useCommandPaletteState();

  // Authenticated users with zero orgs are redirected to onboarding. Reuses the
  // OSS redirect hook so SaaS-style wrappers don't duplicate it.
  const redirectingToOnboarding = useZeroOrgRedirect();

  // Users whose current org never finished onboarding resume at its setup
  // wizard. Only brand-new orgs have a null completion marker (existing orgs are
  // backfilled complete by migration 090), so this is a no-op for everyone else.
  const resumingOnboarding = useOnboardingResumeRedirect();

  // Onboarding/setup routes render full-screen (their own OnboardingShell
  // surface) without the app sidebar chrome. Detected here rather than by moving
  // the route out of `(main)`, which would drop the auth/redirect guards below.
  const onOnboardingRoute = isOnboardingRoute(pathname);

  // Combined loading state - wait for both auth and org to initialize
  const isLoading = authLoading || orgLoading;

  // Redirect to login if auth is required but user is not authenticated
  useEffect(() => {
    if (!authLoading && !authUnavailable && requiresAuth && !isAuthenticated) {
      router.replace(getLoginRedirectPath(pathname, searchParams));
    }
  }, [authLoading, authUnavailable, requiresAuth, isAuthenticated, router, pathname, searchParams]);

  // After OAuth login, check sessionStorage for a pending return_to redirect
  useEffect(() => {
    if (!authLoading && !authUnavailable && isAuthenticated) {
      const pendingReturnTo = sanitizeReturnTo(sessionStorage.getItem(RETURN_TO_STORAGE_KEY));
      sessionStorage.removeItem(RETURN_TO_STORAGE_KEY);
      if (pendingReturnTo) {
        // Backend paths (e.g. /oauth/authorize, /api/v1/auth/cli/callback,
        // or /v1/... when frontend and backend share an origin) need a
        // full page navigation so the reverse proxy forwards them.
        if (isBackendNavigationPath(pendingReturnTo)) {
          window.location.assign(pendingReturnTo);
        } else {
          router.replace(pendingReturnTo);
        }
      }
    }
  }, [authLoading, authUnavailable, isAuthenticated, router]);

  // Show loading state while checking auth and initializing org
  if (isLoading) {
    return (
      <div className="flex h-screen items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (authUnavailable) {
    return <AuthUnavailableState />;
  }

  // If auth required but not authenticated, show nothing (will redirect)
  if (requiresAuth && !isAuthenticated) {
    return null;
  }

  // Zero-org users (or users resuming an incomplete org) are being redirected to
  // onboarding; don't flash the shell.
  if (redirectingToOnboarding || resumingOnboarding) {
    return null;
  }

  // Onboarding/setup routes render their own full-screen surface — no sidebar,
  // banner, or command palette. Guards above still applied.
  if (onOnboardingRoute) {
    return <main className="min-h-screen">{children}</main>;
  }

  const appChrome = (
    <div className="flex h-screen flex-col">
      <EarlyAccessBanner />
      <VerifyEmailBanner />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="flex-1 overflow-auto bg-background bg-brand-dots">{children}</main>
      </div>
      <CommandPalette />
    </div>
  );

  const content = notificationsEnabled ? (
    <NotificationsProvider>{appChrome}</NotificationsProvider>
  ) : (
    appChrome
  );

  return <CommandPaletteContext value={commandPalette}>{content}</CommandPaletteContext>;
}

export default function MainLayout({ children }: MainLayoutProps) {
  return (
    <Suspense
      fallback={
        <div className="flex h-screen items-center justify-center">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
        </div>
      }
    >
      <MainLayoutInner>{children}</MainLayoutInner>
    </Suspense>
  );
}
