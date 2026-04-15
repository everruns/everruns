import type { Metadata } from "next";
import localFont from "next/font/local";
import { cookies } from "next/headers";
import "./globals.css";
import { QueryProvider } from "@/providers/query-provider";
import { FeatureFlagsProvider } from "@/providers/feature-flags-provider";
import { AuthProvider } from "@/providers/auth-provider";
import { OrgProvider } from "@/providers/org-provider";
import { LocaleProvider } from "@/providers/locale-provider";

// Self-hosted via next/font/local to avoid any runtime or build-time dependency
// on fonts.googleapis.com. Downstream apps with strict CSP (font-src 'self')
// won't break.
const caveat = localFont({
  src: "./fonts/Caveat-SemiBold.woff2",
  weight: "600",
  display: "swap",
  variable: "--font-caveat",
});

export const metadata: Metadata = {
  title: "Everruns - AI Agent Management",
  description: "Manage and monitor your AI agents",
};

export default async function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  const cookieStore = await cookies();
  const initialOrgId = cookieStore.get("everruns_org")?.value ?? null;

  return (
    <html lang="en" className={caveat.variable}>
      <body className="font-sans antialiased bg-brand-dots">
        <LocaleProvider>
          <QueryProvider>
            <FeatureFlagsProvider>
              <AuthProvider>
                <OrgProvider initialOrgId={initialOrgId}>{children}</OrgProvider>
              </AuthProvider>
            </FeatureFlagsProvider>
          </QueryProvider>
        </LocaleProvider>
      </body>
    </html>
  );
}
