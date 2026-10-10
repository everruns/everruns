// Navigation model — the single source of which group owns which route.
//
// The sidebar and command palette render these sections; `PageBreadcrumb` derives a page's
// group from the same table (EVE-869). Keeping the definitions here rather than
// in `sidebar.tsx` lets the breadcrumb read them without pulling the whole
// sidebar component tree into every page.
//
// Navigation is grouped by what you do with a thing, not what it is. The
// placement rule and the dismissed alternatives live in
// `knowledge/ui/information-architecture.md`; consult it before adding an
// entity to a group.

import {
  AgentIcon,
  ChatIcon,
  CircuitBreakerIcon,
  DurableIcon,
  EvalsIcon,
  HarnessDomainIcon,
  KnowledgeIcon,
  MemoryIcon,
  ObserverIcon,
  PlaygroundIcon,
  ProviderAccountIcon,
  QueueIcon,
  ReportIcon,
  SandboxIcon,
  SandboxTemplateIcon,
  ScheduleIcon,
  SessionIcon,
  SettingsIcon,
  VirtualUserIcon,
  WorkerIcon,
  WorkflowIcon,
} from "@/components/icons/facet-icons";
import { ArrowRight, Plus } from "lucide-react";
import type { IconComponent } from "@/lib/capability-icons";
import { registryNavigationItems, type RegistryNavigationItem } from "@/lib/registry-navigation";
import type { FeatureFlags } from "@/lib/api/types";

export type NavigationItem = {
  name: string;
  href: string;
  icon: IconComponent;
  keywords?: string[];
  description?: string;
  activePrefix?: string;
  /** Set false to disable the shared hover/focus prefetch in addition to automatic prefetch. */
  prefetch?: boolean;
  flag?: keyof FeatureFlags;
  exact?: boolean;
  /** Keep a primary destination visibly actionable even when another route is open. */
  prominent?: boolean;
  experimental?: boolean;
  warningTooltip?: string;
  /** Minimum organization role required to see this destination. */
  minimumRole?: "admin" | "owner";
  /**
   * Policy id (as served by `/v1/{resource}/config`) the viewer must hold. Callers
   * resolve it with `useNavigationPolicy`; without a checker the item stays hidden.
   */
  policy?: string;
};

export type NavigationSection = {
  /** Stable identifier for sections the shell attaches extra chrome to. */
  id?: string;
  label?: string;
  items: NavigationItem[];
  devOnly?: boolean;
  defaultCollapsed?: boolean;
};

export const defaultChatsNavigation: NavigationItem[] = [
  {
    name: "Chat",
    href: "/chats",
    icon: ChatIcon,
    exact: true,
    prominent: true,
    keywords: ["chats", "global chat", "conversation"],
  },
];

// These links are rendered beneath Chat rather than as peer sidebar destinations.
export const sideChatNavigation: NavigationItem[] = [
  { name: "New side chat", href: "/chats/new", icon: Plus, keywords: ["thread", "conversation"] },
  {
    name: "View all chats",
    href: "/chats/history",
    icon: ArrowRight,
    keywords: ["history", "archived", "threads"],
  },
];

export const defaultOperationalNavigation: NavigationItem[] = [
  {
    name: "Sessions",
    href: "/sessions",
    icon: SessionIcon,
    keywords: ["recordings", "conversation", "transcript"],
  },
];

export const defaultBuildingNavigation: NavigationItem[] = [
  { name: "Agents", href: "/agents", icon: AgentIcon, keywords: ["bot", "assistant"] },
  {
    name: "Playground",
    href: "/playground",
    icon: PlaygroundIcon,
  },
  {
    name: "Harnesses",
    href: "/harnesses",
    icon: HarnessDomainIcon,
    keywords: ["template", "config"],
  },
  {
    name: "Virtual Users",
    href: "/virtual-users",
    icon: VirtualUserIcon,
    keywords: ["persona", "principal", "identity"],
  },
];

export const defaultRegistriesNavigation: NavigationItem[] = [
  ...registryNavigationItems.map<NavigationItem>(
    ({ name, href, icon, keywords }: RegistryNavigationItem) => {
      const flag = href === "/skills" ? "skills" : href === "/plugins" ? "plugins" : undefined;
      return { name, href, icon, keywords, flag, experimental: Boolean(flag) };
    },
  ),
  {
    name: "Knowledge indexes",
    href: "/knowledge-indexes",
    icon: KnowledgeIcon,
    flag: "knowledge",
    experimental: true,
    keywords: ["knowledge", "index", "search", "retrieval"],
  },
  {
    name: "Memory",
    href: "/memory",
    icon: MemoryIcon,
    keywords: ["workspace", "files", "storage"],
    flag: "memory",
    experimental: true,
  },
];

export const defaultQualityNavigation: NavigationItem[] = [
  {
    name: "Evals",
    href: "/evals",
    icon: EvalsIcon,
    keywords: ["evaluation", "test", "benchmark", "score"],
    flag: "evals",
    experimental: true,
  },
  {
    name: "Observers",
    href: "/observers",
    icon: ObserverIcon,
    keywords: ["monitor", "score", "production eval"],
    flag: "observers",
    experimental: true,
  },
  {
    name: "Reports",
    href: "/reports",
    icon: ReportIcon,
    flag: "reports",
    experimental: true,
    keywords: ["analytics", "saved report"],
  },
];

export const defaultBottomNavigation: NavigationItem[] = [
  {
    name: "Settings",
    href: "/settings/organization",
    icon: SettingsIcon,
    activePrefix: "/settings",
    prefetch: false,
    keywords: ["preferences", "config"],
  },
];

// Sandboxes get their own group, collapsed by default: the fleet (what is
// running) and the templates that configure it, side by side.
export const defaultSandboxesNavigation: NavigationItem[] = [
  {
    name: "Fleet",
    href: "/sandboxes",
    icon: SandboxIcon,
    keywords: ["sandboxes", "compute", "daytona", "modal", "containers", "running"],
  },
  {
    name: "Templates",
    href: "/sandbox-templates",
    icon: SandboxTemplateIcon,
    keywords: ["sandbox templates", "environment", "compute", "workspace"],
  },
  {
    name: "Provider Accounts",
    href: "/sandbox-provider-accounts",
    icon: ProviderAccountIcon,
    keywords: ["sandbox credentials", "daytona", "modal", "e2b", "organization accounts"],
    minimumRole: "admin",
  },
];

export const defaultDurableNavigation: NavigationItem[] = [
  { name: "Overview", href: "/durable", icon: DurableIcon, exact: true },
  { name: "Workers", href: "/durable/workers", icon: WorkerIcon },
  { name: "Workflows", href: "/durable/workflows", icon: WorkflowIcon },
  { name: "Queues", href: "/durable/queues", icon: QueueIcon },
  { name: "Schedules", href: "/durable/schedules", icon: ScheduleIcon },
  {
    name: "Circuit Breakers",
    href: "/durable/circuit-breakers",
    icon: CircuitBreakerIcon,
    keywords: ["failure", "resilience"],
  },
];

export const defaultDevNavigation: NavigationItem[] = [
  { name: "Dev Tools", href: "/dev", icon: PlaygroundIcon },
];

export const defaultNavigationSections: NavigationSection[] = [
  { id: "chats", items: defaultChatsNavigation },
  { label: "Building", items: defaultBuildingNavigation },
  { label: "Operational", items: defaultOperationalNavigation },
  { label: "Registers", items: defaultRegistriesNavigation },
  { label: "Quality", items: defaultQualityNavigation },
  { label: "Sandboxes", items: defaultSandboxesNavigation, defaultCollapsed: true },
  { items: defaultBottomNavigation },
  { label: "Durable Execution", items: defaultDurableNavigation, defaultCollapsed: true },
  { label: "Dev", items: defaultDevNavigation, devOnly: true },
];

/** True when `pathname` is `href` or sits beneath it, on a segment boundary. */
function isUnder(pathname: string, href: string): boolean {
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * The label of the sidebar group that owns `pathname`, or `undefined` when the
 * page sits outside a labelled group — Chats and Settings have no group header,
 * so their pages take no group prefix.
 *
 * Matching is longest-href-first so `/agents/all` resolves through `/agents`
 * without `/virtual-users` colliding with it, and `/durable/workers` picks
 * its own entry over `/durable`.
 *
 * This is the only place a page's group is decided: adding a route to a section
 * above gives it the right breadcrumb with no page-level change (EVE-869).
 */
export function navigationGroupForPath(
  pathname: string,
  sections: NavigationSection[] = defaultNavigationSections,
): string | undefined {
  let best: { length: number; label?: string } | undefined;
  for (const section of sections) {
    for (const item of section.items) {
      const href = item.activePrefix ?? item.href;
      if (!isUnder(pathname, href)) continue;
      if (!best || href.length > best.length) {
        best = { length: href.length, label: section.label };
      }
    }
  }
  return best?.label;
}

/** Visibility is shared by the sidebar and command search, including collapsed sections. */
export function visibleNavigationSections(
  sections: NavigationSection[],
  featureFlags: FeatureFlags,
  hasRole: (role: "admin" | "owner") => boolean,
  isDev: boolean,
  can?: (policy: string) => boolean,
): NavigationSection[] {
  return sections
    .filter((section) => !section.devOnly || isDev)
    .map((section) => ({
      ...section,
      items: section.items.filter(
        (item) =>
          (!item.flag || featureFlags[item.flag]) &&
          (!item.minimumRole || hasRole(item.minimumRole)) &&
          (!item.policy || (can?.(item.policy) ?? false)),
      ),
    }))
    .filter((section) => section.items.length > 0);
}

export function isDurableNavigationSection(section: NavigationSection): boolean {
  return section.items.some(
    (item) => item.href === "/durable" || item.href.startsWith("/durable/"),
  );
}
