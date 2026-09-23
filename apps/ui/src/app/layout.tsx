import type { Metadata } from "next";
import { cookies } from "next/headers";
import "./globals.css";
import { QueryProvider } from "@/providers/query-provider";
import { FeatureFlagsProvider } from "@/providers/feature-flags-provider";
import { AuthProvider } from "@/providers/auth-provider";
import { OrgProvider } from "@/providers/org-provider";
import { ProjectProvider } from "@/providers/project-provider";
import { LocaleProvider } from "@/providers/locale-provider";
import { ThemeProvider } from "@/providers/theme-provider";
import { parseThemeMode, THEME_COOKIE, THEME_SCRIPT } from "@/lib/theme";

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
  const initialProjectId = cookieStore.get("everruns_project")?.value ?? null;
  const webMcpOriginTrialToken = process.env.WEBMCP_ORIGIN_TRIAL_TOKEN?.trim();
  const themeMode = parseThemeMode(cookieStore.get(THEME_COOKIE)?.value);

  return (
    // THEME_SCRIPT rewrites the class before hydration when the mode is
    // "system", so the server and client markup legitimately differ here.
    <html lang="en" className={themeMode === "dark" ? "dark" : undefined} suppressHydrationWarning>
      <head>
        {webMcpOriginTrialToken ? (
          <meta httpEquiv="origin-trial" content={webMcpOriginTrialToken} />
        ) : null}
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body className="font-sans antialiased bg-brand-dots">
        <ThemeProvider initialMode={themeMode}>
          <LocaleProvider>
            <QueryProvider>
              <AuthProvider>
                <OrgProvider initialOrgId={initialOrgId}>
                  <FeatureFlagsProvider>
                    <ProjectProvider initialProjectId={initialProjectId}>
                      {children}
                    </ProjectProvider>
                  </FeatureFlagsProvider>
                </OrgProvider>
              </AuthProvider>
            </QueryProvider>
          </LocaleProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
