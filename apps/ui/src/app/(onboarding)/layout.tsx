"use client";

// Onboarding layout — centered, no sidebar. Guards the zero-org onboarding
// surface: requires auth, and bounces users who already belong to an org back
// to the app so onboarding only renders for genuine zero-org users.
import { Suspense, useEffect } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Loader2 } from "lucide-react";
import { AuthUnavailableState } from "@/components/layout/auth-unavailable-state";
import { getLoginRedirectPath, navigateToLogin } from "@/lib/auth-redirect";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";

function LoadingScreen() {
  return (
    <div className="flex h-screen items-center justify-center">
      <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
    </div>
  );
}

function OnboardingLayoutInner({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const {
    config,
    isAuthenticated,
    isLoading: authLoading,
    requiresAuth,
    authUnavailable,
  } = useAuth();
  const { organizations, isLoading: orgLoading } = useOrg();

  const isLoading = authLoading || orgLoading;

  // Redirect to login if auth is required but the user is not authenticated.
  useEffect(() => {
    if (!authLoading && !authUnavailable && requiresAuth && !isAuthenticated) {
      navigateToLogin(
        getLoginRedirectPath(pathname, searchParams, config?.login_origin),
        router.replace,
      );
    }
  }, [
    authLoading,
    authUnavailable,
    requiresAuth,
    isAuthenticated,
    router,
    pathname,
    searchParams,
    config?.login_origin,
  ]);

  // Users that already have an org don't belong on onboarding — send them in.
  useEffect(() => {
    if (!isLoading && isAuthenticated && organizations.length > 0) {
      router.replace("/chats");
    }
  }, [isLoading, isAuthenticated, organizations.length, router]);

  if (isLoading) {
    return <LoadingScreen />;
  }

  // Auth bootstrap failed — show the recoverable error instead of a blank
  // screen (mirrors the main layout).
  if (authUnavailable) {
    return <AuthUnavailableState />;
  }

  // Will redirect to login; render nothing in the meantime.
  if (requiresAuth && !isAuthenticated) {
    return null;
  }

  // Will redirect to the app; render nothing in the meantime.
  if (organizations.length > 0) {
    return null;
  }

  return <>{children}</>;
}

export default function OnboardingLayout({ children }: { children: React.ReactNode }) {
  return (
    <Suspense fallback={<LoadingScreen />}>
      <OnboardingLayoutInner>{children}</OnboardingLayoutInner>
    </Suspense>
  );
}
