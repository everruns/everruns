"use client";

// Sandboxes: one page with tabs, organized like Models. The fleet (every
// Sandbox Sessions ran in), the Sandbox Templates that configure them, and the
// organization's provider accounts they run on. Each tab is its own path
// (`/sandboxes`, `/sandboxes/templates`, `/sandboxes/accounts`); the shell is
// the tabs' layout and each tab's page renders its content.

import type { ReactNode } from "react";
import { usePathname } from "next/navigation";
import { Box, KeyRound, Layers, Plus } from "lucide-react";
import { SandboxIcon } from "@/components/icons/facet-icons";
import { LinkButton } from "@/components/ui/button";
import { PageBreadcrumb, PageContainer, PageMasthead, SectionTabs } from "@/components/layout";
import { useSandboxTemplates } from "@/hooks";
import { useSandboxFleetStats } from "@/hooks/use-sandbox-fleet";
import { tabFromPath, tabHref } from "@/lib/tab-routes";
import { useOrg } from "@/providers/org-provider";

const TABS = ["sandboxes", "templates", "accounts"] as const;
type Tab = (typeof TABS)[number];
const tabPath = (tab: Tab) => tabHref("/sandboxes", tab, "sandboxes");

const DESCRIPTIONS: Record<Tab, string> = {
  sandboxes:
    "Every Sandbox your Sessions ran in, across providers: running, paused, lost and deleted.",
  templates: "Reusable, versioned configuration for the primary Sandbox each Session receives.",
  accounts:
    "Organization-owned credentials that managed Sandbox Templates can use for every Session.",
};

export function SandboxesShell({ children }: { children: ReactNode }) {
  const tab = tabFromPath(usePathname(), "/sandboxes", TABS, "sandboxes");
  const { hasRole } = useOrg();
  // Provider accounts hold organization credentials; only admins manage them.
  const isAdmin = hasRole("admin");
  const stats = useSandboxFleetStats({ provider: "all", search: "" });
  const templates = useSandboxTemplates();
  const live = stats.data?.live_by_provider.reduce((sum, entry) => sum + entry.count, 0);

  const tabs = [
    {
      value: "sandboxes",
      href: tabPath("sandboxes"),
      label: "Sandboxes",
      icon: <Box className="icon-sharp size-3.5" />,
      count: live,
    },
    {
      value: "templates",
      href: tabPath("templates"),
      label: "Templates",
      icon: <Layers className="icon-sharp size-3.5" />,
      count: templates.data?.length,
    },
    ...(isAdmin || tab === "accounts"
      ? [
          {
            value: "accounts",
            href: tabPath("accounts"),
            label: "Provider accounts",
            icon: <KeyRound className="icon-sharp size-3.5" />,
          },
        ]
      : []),
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Sandboxes" }]} />
      <PageMasthead
        icon={<SandboxIcon />}
        title="Sandboxes"
        description={DESCRIPTIONS[tab]}
        actions={
          tab === "templates" ? (
            <LinkButton variant="accent" href="/sandboxes/templates/new">
              <Plus className="size-4" /> New Sandbox Template
            </LinkButton>
          ) : undefined
        }
      />
      <SectionTabs aria-label="Sandboxes sections" value={tab} items={tabs} />
      {children}
    </PageContainer>
  );
}
