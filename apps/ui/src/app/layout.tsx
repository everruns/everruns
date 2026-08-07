import type { Metadata } from "next";
import { cookies } from "next/headers";
import "./globals.css";
import { QueryProvider } from "@/providers/query-provider";
import { FeatureFlagsProvider } from "@/providers/feature-flags-provider";
import { AuthProvider } from "@/providers/auth-provider";
import { OrgProvider } from "@/providers/org-provider";
import { LocaleProvider } from "@/providers/locale-provider";

// Default title — individual pages override via usePageTitle.
// See knowledge/foundations/code-organization.md (Page Titles) for the format.
export const metadata: Metadata = {
  title: "Everruns",
  description: "Manage and monitor your AI agents",
};

export default async function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  const cookieStore = await cookies();
  const initialOrgId = cookieStore.get("everruns_org")?.value ?? null;
  const webMcpOriginTrialToken = process.env.WEBMCP_ORIGIN_TRIAL_TOKEN?.trim();

  return (
    <html lang="en">
      {webMcpOriginTrialToken ? (
        <head>
          <meta httpEquiv="origin-trial" content={webMcpOriginTrialToken} />
        </head>
      ) : null}
      <body className="font-sans antialiased bg-brand-dots">
        <LocaleProvider>
          <QueryProvider>
            <AuthProvider>
              <OrgProvider initialOrgId={initialOrgId}>
                <FeatureFlagsProvider>{children}</FeatureFlagsProvider>
              </OrgProvider>
            </AuthProvider>
          </QueryProvider>
        </LocaleProvider>
      </body>
    </html>
  );
}
