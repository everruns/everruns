"use client";

// Main app layout with sidebar, auth guard, and global command palette
import { Suspense, useEffect } from "react";
import { useRouter, usePathname, useSearchParams } from "next/navigation";
import { Sidebar } from "@/components/layout/sidebar";
import { CommandPalette } from "@/components/command-palette";
import { CommandPaletteContext, useCommandPaletteState } from "@/hooks/use-command-palette";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { Loader2 } from "lucide-react";

interface MainLayoutProps {
  children: React.ReactNode;
}

function MainLayoutInner({ children }: MainLayoutProps) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { isAuthenticated, isLoading: authLoading, requiresAuth } = useAuth();
  const { isLoading: orgLoading } = useOrg();
  const commandPalette = useCommandPaletteState();

  // Combined loading state - wait for both auth and org to initialize
  const isLoading = authLoading || orgLoading;

  // Redirect to login if auth is required but user is not authenticated
  useEffect(() => {
    if (!authLoading && requiresAuth && !isAuthenticated) {
      // Preserve current location so login can redirect back
      const currentUrl = pathname + (searchParams.toString() ? `?${searchParams.toString()}` : "");
      const returnTo =
        currentUrl !== "/dashboard" ? `?return_to=${encodeURIComponent(currentUrl)}` : "";
      router.replace(`/login${returnTo}`);
    }
  }, [authLoading, requiresAuth, isAuthenticated, router, pathname, searchParams]);

  // After OAuth login, check sessionStorage for a pending return_to redirect
  useEffect(() => {
    if (!authLoading && isAuthenticated) {
      const pendingReturnTo = sessionStorage.getItem("everruns_return_to");
      if (pendingReturnTo) {
        sessionStorage.removeItem("everruns_return_to");
        router.replace(pendingReturnTo);
      }
    }
  }, [authLoading, isAuthenticated, router]);

  // Show loading state while checking auth and initializing org
  if (isLoading) {
    return (
      <div className="flex h-screen items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  // If auth required but not authenticated, show nothing (will redirect)
  if (requiresAuth && !isAuthenticated) {
    return null;
  }

  return (
    <CommandPaletteContext value={commandPalette}>
      <div className="flex h-screen">
        <Sidebar />
        <main className="flex-1 overflow-auto bg-background bg-brand-dots">{children}</main>
        <CommandPalette />
      </div>
    </CommandPaletteContext>
  );
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
