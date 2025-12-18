"use client";

// Main app layout with sidebar and auth guard
import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { Sidebar } from "@/components/layout/sidebar";
import { useAuth } from "@/providers/auth-provider";
import { Loader2 } from "lucide-react";

interface MainLayoutProps {
  children: React.ReactNode;
}

export default function MainLayout({ children }: MainLayoutProps) {
  const router = useRouter();
  const { isAuthenticated, isLoading, requiresAuth } = useAuth();

  // Redirect to login if auth is required but user is not authenticated
  useEffect(() => {
    if (!isLoading && requiresAuth && !isAuthenticated) {
      router.replace("/login");
    }
  }, [isLoading, requiresAuth, isAuthenticated, router]);

  // Show loading state while checking auth
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
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex-1 overflow-auto bg-background">{children}</main>
    </div>
  );
}
