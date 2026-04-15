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
 * - orgActions.createOrg: override "Create Organisation" click handler
 * - extraSections: append additional sections (billing, usage, etc.)
 */
"use client";

import packageJson from "../../../package.json";
import Link from "next/link";
import Image from "next/image";
import { usePathname } from "next/navigation";
import {
  Boxes,
  BookOpen,
  Calendar,
  ClipboardCheck,
  FlaskConical,
  LayoutDashboard,
  ListTodo,
  MessageCircle,
  MessageSquare,
  Puzzle,
  Rocket,
  Search,
  Server,
  Settings,
  Shield,
  UserRound,
  Workflow,
  Cog,
} from "lucide-react";
import { capabilityIconMap } from "@/lib/capability-icons";
import { useCommandPalette } from "@/hooks/use-command-palette";
import { useLlmProviders } from "@/hooks/use-llm-providers";
import { useAuth } from "@/providers/auth-provider";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import type { FeatureFlags } from "@/lib/api/types";
import { SidebarNavigation } from "./sidebar-navigation";
import { SidebarOrganizationMenu } from "./sidebar-organization-menu";
import { SidebarUserMenu } from "./sidebar-user-menu";

const { version } = packageJson;

export type NavigationItem = {
  name: string;
  href: string;
  icon: React.ComponentType<{ className?: string }>;
  flag?: keyof FeatureFlags;
  exact?: boolean;
  experimental?: boolean;
  warningTooltip?: string;
};

export type NavigationSection = {
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
}

export const defaultTopNavigation: NavigationItem[] = [
  { name: "Chat", href: "/chat", icon: MessageCircle, flag: "global_chat", experimental: true },
  { name: "Dashboard", href: "/dashboard", icon: LayoutDashboard },
  { name: "Sessions", href: "/sessions", icon: MessageSquare },
];

export const defaultBuildingBlocksNavigation: NavigationItem[] = [
  { name: "Harnesses", href: "/harnesses", icon: Shield },
  { name: "Agents", href: "/agents", icon: Boxes },
  { name: "Agent Identities", href: "/agent-identities", icon: UserRound },
  { name: "Skills", href: "/skills", icon: BookOpen },
  { name: "Capabilities", href: "/capabilities", icon: Puzzle },
  { name: "MCP Servers", href: "/mcp-servers", icon: capabilityIconMap.mcp },
  { name: "Apps", href: "/apps", icon: Rocket, flag: "apps", experimental: true },
  { name: "Evals", href: "/evals", icon: ClipboardCheck, flag: "evals", experimental: true },
];

export const defaultBottomNavigation: NavigationItem[] = [
  { name: "Settings", href: "/settings", icon: Settings },
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
  { items: defaultTopNavigation },
  { label: "Building Blocks", items: defaultBuildingBlocksNavigation },
  { items: defaultBottomNavigation },
  { label: "Durable Execution", items: defaultDurableNavigation, defaultCollapsed: true },
  { label: "Dev", items: defaultDevNavigation, devOnly: true },
];

export function Sidebar({ config }: { config?: Partial<SidebarConfig> }) {
  const pathname = usePathname();
  const {
    user,
    requiresAuth,
    logout,
    logoutPending,
    createOrganization: createOrgOverride,
  } = useAuth();
  const featureFlags = useFeatureFlags();
  const {
    data: llmProviders,
    isLoading: llmProvidersLoading,
    isError: llmProvidersError,
  } = useLlmProviders();
  const { setOpen: openCommandPalette } = useCommandPalette();

  const llmProvidersReady = !llmProvidersLoading && !llmProvidersError;
  const shouldShowChatWarning = llmProvidersReady && (!llmProviders || llmProviders.length === 0);

  const baseSections = config?.navigation ?? defaultNavigationSections;
  const sections = shouldShowChatWarning
    ? baseSections.map((section) => ({
        ...section,
        items: section.items.map((item) =>
          item.href === "/chat"
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
    <div className="flex h-full w-60 flex-col border-r border-border/70 bg-background">
      <div className="flex h-14 items-center justify-between border-b border-border/70 bg-card px-4">
        <Link href="/dashboard" className="flex items-center gap-2">
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

      <SidebarNavigation sections={allSections} pathname={pathname} featureFlags={featureFlags} />

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
          version={version}
        />
      </div>
    </div>
  );
}
