import type { Metadata } from "next";
import "./globals.css";
import { QueryProvider } from "@/providers/query-provider";
import { FeatureFlagsProvider } from "@/providers/feature-flags-provider";
import { AuthProvider } from "@/providers/auth-provider";
import { OrgProvider } from "@/providers/org-provider";

export const metadata: Metadata = {
  title: "Everruns - AI Agent Management",
  description: "Manage and monitor your AI agents",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className="font-sans antialiased bg-brand-dots">
        <QueryProvider>
          <FeatureFlagsProvider>
            <AuthProvider>
              <OrgProvider>{children}</OrgProvider>
            </AuthProvider>
          </FeatureFlagsProvider>
        </QueryProvider>
      </body>
    </html>
  );
}
