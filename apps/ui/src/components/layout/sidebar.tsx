/**
 * Composable sidebar with extension points.
 *
 * Design: The sidebar accepts an optional SidebarConfig to override navigation
 * sections, org actions, and inject extra sections. When no config is passed,
 * behavior is identical to the original hardcoded sidebar. This enables SaaS
 * forks to customize the sidebar in ~15 lines instead of duplicating 300+.
 *
 * Extension points:
 * - navigation: replace the default navigation sections entirely
 * - orgActions.createOrg: override "Create Organization" click handler
 * - extraSections: append additional sections (billing, usage, etc.)
 * - profileMenu.items: append items inside the authenticated profile menu
 */
"use client";

import packageJson from "../../../package.json";
import Link from "next/link";
import Image from "next/image";
import { usePathname } from "next/navigation";
import { useState } from "react";
import { Search, Menu } from "lucide-react";
import {
  defaultBottomNavigation,
  defaultBuildingNavigation,
  defaultChatsNavigation,
  defaultDevNavigation,
  defaultDurableNavigation,
  defaultNavigationSections,
  defaultOperationalNavigation,
  defaultQualityNavigation,
  defaultRegistriesNavigation,
} from "@/lib/navigation";
import type { NavigationItem, NavigationSection } from "@/lib/navigation";

// The navigation model moved to `@/lib/navigation` so `PageBreadcrumb` can read
// which group owns a route without importing the sidebar (EVE-869). Re-exported
// here because the sidebar has been its public home.
export type { NavigationItem, NavigationSection };
export {
  defaultBottomNavigation,
  defaultBuildingNavigation,
  defaultChatsNavigation,
  defaultDevNavigation,
  defaultDurableNavigation,
  defaultNavigationSections,
  defaultOperationalNavigation,
  defaultQualityNavigation,
  defaultRegistriesNavigation,
};
import { useCommandPalette } from "@/hooks/use-command-palette";
import { useProviders } from "@/hooks/use-providers";
import { usePolicies } from "@/hooks/use-policies";
import { useAuth } from "@/providers/auth-provider";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import { SidebarNavigation } from "./sidebar-navigation";
import { SidebarChatThreads } from "./sidebar-chat-threads";
import { SidebarOrganizationMenu } from "./sidebar-organization-menu";
import { SidebarUserMenu } from "./sidebar-user-menu";
import type { SidebarUserMenuItemsRenderer } from "./sidebar-user-menu";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Drawer, DrawerContent, DrawerTitle } from "@/components/ui/drawer";

const { version } = packageJson;

export interface SidebarConfig {
  navigation?: NavigationSection[];
  orgActions?: {
    createOrg?: () => void;
  };
  extraSections?: NavigationSection[];
  profileMenu?: {
    items?: SidebarUserMenuItemsRenderer;
  };
}

function isDurableSection(section: NavigationSection) {
  return section.items.some(
    (item) => item.href === "/durable" || item.href.startsWith("/durable/"),
  );
}

export function Sidebar({
  config,
  className,
}: {
  config?: Partial<SidebarConfig>;
  className?: string;
}) {
  const pathname = usePathname();
  const {
    user,
    requiresAuth,
    logout,
    logoutPending,
    createOrganization: createOrgOverride,
  } = useAuth();
  const featureFlags = useFeatureFlags();
  const { data: providers, isLoading: providersLoading, isError: providersError } = useProviders();
  const durablePolicies = usePolicies("durable");
  const { setOpen: openCommandPalette } = useCommandPalette();

  const providersReady = !providersLoading && !providersError;
  const shouldShowChatWarning = providersReady && (!providers || providers.length === 0);

  const durableAllowed = durablePolicies.data ? durablePolicies.can("durable.view") : false;
  const baseSections = config?.navigation
    ? config.navigation
    : defaultNavigationSections.filter((section) => !isDurableSection(section) || durableAllowed);
  const sections = shouldShowChatWarning
    ? baseSections.map((section) => ({
        ...section,
        items: section.items.map((item) =>
          item.href === "/chats"
            ? {
                ...item,
                warningTooltip:
                  "No LLM provider configured. Set one up in Settings → Providers to use Chat.",
              }
            : item,
        ),
      }))
    : baseSections;
  const allSections = config?.extraSections ? [...sections, ...config.extraSections] : sections;

  const handleCreateOrg = config?.orgActions?.createOrg ?? createOrgOverride ?? (() => {});
  const useDefaultCreateOrgDialog = !config?.orgActions?.createOrg && !createOrgOverride;

  return (
    <div
      data-slot="app-sidebar"
      className={cn("flex h-full w-60 flex-col border-r border-border/70 bg-background", className)}
    >
      <div className="flex h-14 items-center justify-between border-b border-border/70 bg-card px-4">
        <Link href="/dashboard" prefetch={false} className="flex items-center gap-2">
          <Image src="/logo.svg" alt="Everruns" width={28} height={28} />
          <span className="text-base font-semibold tracking-[-0.02em]">Everruns</span>
        </Link>
      </div>

      <SidebarOrganizationMenu
        onCreateOrg={handleCreateOrg}
        useDefaultCreateOrgDialog={useDefaultCreateOrgDialog}
      />

      <div role="search" aria-label="Sidebar search" className="px-2.5 py-2">
        <button
          type="button"
          onClick={() => openCommandPalette(true)}
          className="flex w-full items-center gap-2 border border-input bg-transparent px-2.5 py-1.5 text-[13px] font-medium text-muted-foreground transition-colors hover:border-border hover:bg-card hover:text-foreground"
        >
          <Search className="h-3.5 w-3.5 shrink-0" />
          <span className="flex-1 text-left">Search...</span>
          <kbd className="hidden h-4 items-center gap-0.5 border bg-muted px-1.5 font-mono text-[10px] font-medium sm:inline-flex">
            <span className="text-[11px]">⌘</span>K
          </kbd>
        </button>
      </div>

      <SidebarNavigation
        sections={allSections}
        pathname={pathname}
        featureFlags={featureFlags}
        renderSectionExtra={(section) =>
          section.id === "chats" ? <SidebarChatThreads pathname={pathname} /> : null
        }
      />

      <div
        role="contentinfo"
        aria-label="Sidebar footer"
        className="border-t border-border/70 p-2.5"
      >
        <SidebarUserMenu
          requiresAuth={requiresAuth}
          user={user ?? null}
          logout={logout}
          logoutPending={logoutPending}
          renderExtraItems={config?.profileMenu?.items}
          version={version}
        />
      </div>
    </div>
  );
}

export function MobileSidebar({ config }: { config?: Partial<SidebarConfig> }) {
  const [open, setOpen] = useState(false);

  return (
    <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border/70 bg-card px-3 md:hidden">
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label="Open navigation"
        onClick={() => setOpen(true)}
      >
        <Menu className="size-5" />
      </Button>
      <Link href="/dashboard" prefetch={false} className="flex items-center gap-2">
        <Image src="/logo.svg" alt="" width={24} height={24} />
        <span className="text-sm font-semibold tracking-[-0.02em]">Everruns</span>
      </Link>
      <Drawer open={open} onOpenChange={setOpen}>
        <DrawerContent
          side="left"
          className="w-60 max-w-none gap-0 p-0 sm:max-w-none"
          onClick={(event) => {
            if ((event.target as Element).closest("a")) setOpen(false);
          }}
        >
          <DrawerTitle className="sr-only">Navigation</DrawerTitle>
          <Sidebar config={config} className="w-full border-r-0" />
        </DrawerContent>
      </Drawer>
    </div>
  );
}
