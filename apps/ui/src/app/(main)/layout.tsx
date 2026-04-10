"use client";

// Main app layout with sidebar, auth guard, and global command palette
import { Suspense, useEffect } from "react";
import { useRouter, usePathname, useSearchParams } from "next/navigation";
import { AlertTriangle, Loader2 } from "lucide-react";
import { Sidebar } from "@/components/layout/sidebar";
import { CommandPalette } from "@/components/command-palette";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { CommandPaletteContext, useCommandPaletteState } from "@/hooks/use-command-palette";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { NotificationsProvider } from "@/providers/notifications-provider";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

interface MainLayoutProps {
  children: React.ReactNode;
}

function AuthUnavailableState() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4">
      <Card className="w-full max-w-lg">
        <CardHeader className="items-center text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted">
            <AlertTriangle className="h-6 w-6 text-destructive" />
          </div>
          <CardTitle>Authentication unavailable</CardTitle>
          <CardDescription>
            We couldn&apos;t verify your authentication state. Protected routes stay blocked until
            auth bootstrap succeeds.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex justify-center">
          <Button onClick={() => window.location.reload()}>Retry</Button>
        </CardContent>
      </Card>
    </div>
  );
}

function MainLayoutInner({ children }: MainLayoutProps) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { isAuthenticated, isLoading: authLoading, requiresAuth, authUnavailable } = useAuth();
  const { isLoading: orgLoading } = useOrg();
  const notificationsEnabled = useFeatureFlag("notifications");
  const commandPalette = useCommandPaletteState();

  // Combined loading state - wait for both auth and org to initialize
  const isLoading = authLoading || orgLoading;

  // Redirect to login if auth is required but user is not authenticated
  useEffect(() => {
    if (!authLoading && !authUnavailable && requiresAuth && !isAuthenticated) {
      // Preserve current location so login can redirect back
      const currentUrl = pathname + (searchParams.toString() ? `?${searchParams.toString()}` : "");
      const returnTo =
        currentUrl !== "/dashboard" ? `?return_to=${encodeURIComponent(currentUrl)}` : "";
      router.replace(`/login${returnTo}`);
    }
  }, [authLoading, authUnavailable, requiresAuth, isAuthenticated, router, pathname, searchParams]);

  // After OAuth login, check sessionStorage for a pending return_to redirect
  useEffect(() => {
    if (!authLoading && !authUnavailable && isAuthenticated) {
      const pendingReturnTo = sessionStorage.getItem("everruns_return_to");
      if (pendingReturnTo) {
        sessionStorage.removeItem("everruns_return_to");
        // Backend paths (e.g. /oauth/authorize) need a full page navigation
        if (pendingReturnTo.startsWith("/oauth/") || pendingReturnTo.startsWith("/api/")) {
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

  const appChrome = (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex-1 overflow-auto bg-background bg-brand-dots">{children}</main>
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
