"use client";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import { AuthProvider } from "@/providers/auth-provider";
import { OrgProvider } from "@/providers/org-provider";
import { FeatureFlagsProvider } from "@/providers/feature-flags-provider";
import { McpAppProvider } from "./provider";

export function ApplicationProviders({
  children,
  initialOrgId,
}: {
  children: ReactNode;
  initialOrgId: string | null;
}) {
  const pathname = usePathname();
  if (pathname === "/mcp-app" || pathname.startsWith("/mcp-app/"))
    return <McpAppProvider>{children}</McpAppProvider>;
  return (
    <AuthProvider>
      <OrgProvider initialOrgId={initialOrgId}>
        <FeatureFlagsProvider>{children}</FeatureFlagsProvider>
      </OrgProvider>
    </AuthProvider>
  );
}
