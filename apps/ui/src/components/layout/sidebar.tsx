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
import {
  Boxes,
  Brain,
  Calendar,
  ChartColumn,
  ClipboardCheck,
  FlaskConical,
  Library,
  ListTodo,
  MessageCircle,
  MessageSquare,
  Rocket,
  Search,
  Server,
  Settings,
  Shield,
  Telescope,
  UserRound,
  Workflow,
  Cog,
  Menu,
} from "lucide-react";
import type { IconComponent } from "@/lib/capability-icons";
import { registryNavigationItems } from "@/lib/registry-navigation";
import { useCommandPalette } from "@/hooks/use-command-palette";
import { useProviders } from "@/hooks/use-providers";
import { usePolicies } from "@/hooks/use-policies";
import { useAuth } from "@/providers/auth-provider";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import type { FeatureFlags } from "@/lib/api/types";
import { SidebarNavigation } from "./sidebar-navigation";
import { SidebarChatThreads } from "./sidebar-chat-threads";
import { SidebarOrganizationMenu } from "./sidebar-organization-menu";
import { SidebarUserMenu } from "./sidebar-user-menu";
import type { SidebarUserMenuItemsRenderer } from "./sidebar-user-menu";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Drawer, DrawerContent, DrawerTitle } from "@/components/ui/drawer";

const { version } = packageJson;

export type NavigationItem = {
  name: string;
  href: string;
  icon: IconComponent;
  activePrefix?: string;
  /** Set false to disable the shared hover/focus prefetch in addition to automatic prefetch. */
  prefetch?: boolean;
  flag?: keyof FeatureFlags;
  exact?: boolean;
  experimental?: boolean;
  warningTooltip?: string;
};

export type NavigationSection = {
  /** Stable identifier for sections the shell attaches extra chrome to. */
  id?: string;
  label?: string;
  items: NavigationItem[];
  devOnly?: boolean;
  defaultCollapsed?: boolean;
};

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

/**
 * Navigation is grouped by what you do with a thing, not what it is. The placement
 * rule and the dismissed alternatives live in `knowledge/ui/information-architecture.md`;
 * consult it before adding an entity to a group.
 */
export const defaultChatsNavigation: NavigationItem[] = [
  { name: "Chats", href: "/chats", icon: MessageCircle, flag: "global_chat", experimental: true },
];

export const defaultOperationalNavigation: NavigationItem[] = [
  { name: "Sessions", href: "/sessions", icon: MessageSquare },
  { name: "Reports", href: "/reports", icon: ChartColumn },
];

export const defaultBuildingNavigation: NavigationItem[] = [
  { name: "Agents", href: "/agents", icon: Boxes },
  { name: "Harnesses", href: "/harnesses", icon: Shield },
  { name: "Identities", href: "/agent-identities", icon: UserRound },
  {
    name: "Knowledge indexes",
    href: "/knowledge-indexes",
    icon: Library,
    flag: "knowledge",
    experimental: true,
  },
  { name: "Memory", href: "/memory", icon: Brain, flag: "memory", experimental: true },
  { name: "Apps", href: "/apps", icon: Rocket },
];

export const defaultRegistriesNavigation: NavigationItem[] = registryNavigationItems.map(
  ({ name, href, icon }) => {
    const flag = href === "/skills" ? "skills" : href === "/plugins" ? "plugins" : undefined;
    return { name, href, icon, flag, experimental: Boolean(flag) };
  },
);

export const defaultQualityNavigation: NavigationItem[] = [
  { name: "Evals", href: "/evals", icon: ClipboardCheck, flag: "evals", experimental: true },
  {
    name: "Observers",
    href: "/observers",
    icon: Telescope,
    flag: "observers",
    experimental: true,
  },
];

export const defaultBottomNavigation: NavigationItem[] = [
  {
    name: "Settings",
    href: "/settings/organization",
    icon: Settings,
    activePrefix: "/settings",
    prefetch: false,
  },
];

export const defaultDurableNavigation: NavigationItem[] = [
  { name: "Overview", href: "/durable", icon: Cog, exact: true },
  { name: "Workers", href: "/durable/workers", icon: Server },
  { name: "Workflows", href: "/durable/workflows", icon: Workflow },
  { name: "Queues", href: "/durable/queues", icon: ListTodo },
  { name: "Schedules", href: "/durable/schedules", icon: Calendar },
];

export const defaultDevNavigation: NavigationItem[] = [
  { name: "Dev Tools", href: "/dev", icon: FlaskConical },
];

export const defaultNavigationSections: NavigationSection[] = [
  { id: "chats", items: defaultChatsNavigation },
  { label: "Operational", items: defaultOperationalNavigation },
  { label: "Building", items: defaultBuildingNavigation },
  { label: "Registries", items: defaultRegistriesNavigation },
  { label: "Quality", items: defaultQualityNavigation },
  { items: defaultBottomNavigation },
  { label: "Durable Execution", items: defaultDurableNavigation, defaultCollapsed: true },
  { label: "Dev", items: defaultDevNavigation, devOnly: true },
];

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
          section.id === "chats" && featureFlags.global_chat ? (
            <SidebarChatThreads pathname={pathname} />
          ) : null
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
