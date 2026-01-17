import type { Metadata } from "next";
import "./globals.css";
import { QueryProvider } from "@/providers/query-provider";
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
          <AuthProvider>
            <OrgProvider>{children}</OrgProvider>
          </AuthProvider>
        </QueryProvider>
      </body>
    </html>
  );
}
