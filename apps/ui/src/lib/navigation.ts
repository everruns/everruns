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
  HealthIcon,
  KnowledgeIcon,
  MemoryIcon,
  OrganizationIcon,
  ObserverIcon,
  PlaygroundIcon,
  QueueIcon,
  ReportIcon,
  RoomIcon,
  SandboxIcon,
  ScheduleIcon,
  SessionIcon,
  SettingsIcon,
  VirtualUserIcon,
  WorkerIcon,
  WorkflowIcon,
} from "@/components/icons/facet-icons";
import { ArrowRight, Plus } from "lucide-react";
import type { IconComponent } from "@/lib/capability-icons";
import { registryNavigationByHref, type RegistryNavigationItem } from "@/lib/registry-navigation";
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
  // Main chat is the person's permanent Platform Chat. From any app page it
  // opens beside that page (knowledge/ui/chat-mode.md); /chats is full screen.
  {
    name: "Main chat",
    description: "Talk with your personal assistant. It opens beside the page you are on.",
    href: "/chats",
    icon: ChatIcon,
    exact: true,
    prominent: true,
    keywords: ["chat", "chats", "global chat", "conversation", "coordinator"],
  },
];

// These links are rendered beneath Chat rather than as peer sidebar destinations.
export const sideChatNavigation: NavigationItem[] = [
  {
    name: "New side chat",
    href: "/chats/new",
    icon: Plus,
    keywords: ["subchat", "conversation", "side chat", "new chat"],
  },
  {
    name: "View all chats",
    href: "/chats/history",
    icon: ArrowRight,
    keywords: ["history", "archived", "threads"],
  },
];

export const defaultOverviewNavigation: NavigationItem[] = [
  // Overview (route /home) sits under the chats with no group header: it is the
  // org's home, what needs a person, with the count of waiting items on its
  // sidebar entry (knowledge/ui/org-home.md).
  {
    name: "Overview",
    description: "What needs attention in this organization, and how its agents ran today.",
    href: "/home",
    icon: OrganizationIcon,
    keywords: ["home", "dashboard", "overview", "attention", "health", "status"],
  },
];

export const defaultBuildNavigation: NavigationItem[] = [
  {
    name: "Agents",
    description: "Build and configure your AI assistants.",
    href: "/agents",
    icon: AgentIcon,
    keywords: ["bot", "assistant"],
  },
  {
    name: "Playground",
    description: "Try agents in shared test conversations.",
    href: "/playground",
    icon: PlaygroundIcon,
  },
  {
    name: "Extractions",
    description: "Extract typed values and evidence using your provider account.",
    href: "/extractions",
    icon: ReportIcon,
    flag: "extractions",
    policy: "model.manage",
    keywords: ["extract", "structured data", "evidence"],
  },
];

export const defaultActivityNavigation: NavigationItem[] = [
  {
    name: "Sessions",
    description: "Review recorded conversations and execution traces.",
    href: "/sessions",
    icon: SessionIcon,
    keywords: ["recordings", "conversation", "transcript"],
  },
  // Rooms are a building block like sessions: apps make them, the console
  // lists them for inspection (knowledge/runtime-resources/rooms.md).
  {
    name: "Rooms",
    description: "Inspect the rooms where people and agents talk together.",
    href: "/rooms",
    icon: RoomIcon,
    keywords: ["room", "board", "channel", "conversation", "group", "styk", "facilitator"],
  },
  {
    name: "Monitoring",
    description: "Inspect running sessions, actors and leases.",
    href: "/monitoring",
    icon: DurableIcon,
    flag: "monitoring",
    experimental: true,
    keywords: ["execution", "actors", "leases", "monitor"],
  },
];

function registryItem(href: keyof typeof registryNavigationByHref): NavigationItem {
  const { name, icon, keywords, description }: RegistryNavigationItem =
    registryNavigationByHref[href];
  const flag = href === "/skills" ? "skills" : href === "/plugins" ? "plugins" : undefined;
  return { name, href, icon, keywords, description, flag, experimental: Boolean(flag) };
}

// Blocks: everything agents are made from and run on. What agents use by name
// comes first; the provider-backed pages (Models, Web search, Sandboxes), each
// with its own accounts tab, close the group.
export const defaultBlocksNavigation: NavigationItem[] = [
  {
    name: "Harnesses",
    description: "Reusable instructions and tools for agents.",
    href: "/harnesses",
    icon: HarnessDomainIcon,
    keywords: ["template", "config"],
  },
  registryItem("/capabilities"),
  registryItem("/skills"),
  registryItem("/plugins"),
  {
    name: "Knowledge",
    description: "Organize collections, sources and searchable knowledge for agents.",
    href: "/knowledge",
    icon: KnowledgeIcon,
    flag: "knowledge",
    experimental: true,
    keywords: ["knowledge", "index", "search", "retrieval"],
  },
  {
    name: "Memory",
    description: "Browse the persistent memory available to agents.",
    href: "/memory",
    icon: MemoryIcon,
    keywords: ["workspace", "files", "storage"],
    flag: "memory",
    experimental: true,
  },
  {
    name: "Virtual Users",
    description: "Manage the identities and personas agents interact with.",
    href: "/virtual-users",
    icon: VirtualUserIcon,
    keywords: ["persona", "principal", "identity"],
  },
  registryItem("/models"),
  registryItem("/web-search"),
  // One page with tabs, like Models: the fleet (what is running), the
  // templates that configure it, and the provider accounts it runs on.
  {
    name: "Sandboxes",
    description: "Manage agent environments, templates and providers.",
    href: "/sandboxes",
    icon: SandboxIcon,
    keywords: [
      "fleet",
      "compute",
      "daytona",
      "modal",
      "boxd",
      "isolated sandbox",
      "kubernetes",
      "containers",
      "running",
      "sandbox templates",
      "environment",
      "workspace",
      "provider accounts",
      "sandbox credentials",
    ],
  },
];

export const defaultQualityNavigation: NavigationItem[] = [
  {
    name: "Evals",
    description: "Test and compare how well agents perform.",
    href: "/evals",
    icon: EvalsIcon,
    keywords: ["evaluation", "test", "benchmark", "score"],
    flag: "evals",
    experimental: true,
  },
  {
    name: "Observers",
    description: "Monitor agent behavior and quality over time.",
    href: "/observers",
    icon: ObserverIcon,
    keywords: ["monitor", "score", "production eval"],
    flag: "observers",
    experimental: true,
  },
];

export const defaultBottomNavigation: NavigationItem[] = [
  {
    name: "Settings",
    description: "Manage your organization, team and preferences.",
    href: "/settings/organization",
    icon: SettingsIcon,
    activePrefix: "/settings",
    prefetch: false,
    keywords: ["preferences", "config"],
  },
];

export const defaultDurableNavigation: NavigationItem[] = [
  {
    name: "Health",
    description: "Check that every part of the install is set up right and working.",
    href: "/system/health",
    icon: HealthIcon,
    keywords: ["doctor", "status", "diagnostics", "setup", "install", "versions"],
  },
  {
    name: "Monitoring",
    description: "Inspect running sessions, actors and leases.",
    href: "/durable",
    icon: DurableIcon,
    exact: true,
    keywords: ["fleet", "workers", "actors", "leases", "monitor"],
  },
  {
    name: "Workers",
    description: "Inspect the workers that execute agent tasks.",
    href: "/durable/workers",
    icon: WorkerIcon,
  },
  {
    name: "Workflows",
    description: "Inspect durable workflow execution.",
    href: "/durable/workflows",
    icon: WorkflowIcon,
  },
  {
    name: "Queues",
    description: "Inspect queued work and task delivery.",
    href: "/durable/queues",
    icon: QueueIcon,
  },
  {
    name: "Schedules",
    description: "Inspect scheduled work and upcoming runs.",
    href: "/durable/schedules",
    icon: ScheduleIcon,
  },
  {
    name: "Circuit Breakers",
    description: "Inspect failure protection and recovery state.",
    href: "/durable/circuit-breakers",
    icon: CircuitBreakerIcon,
    keywords: ["failure", "resilience"],
  },
];

export const defaultDevNavigation: NavigationItem[] = [
  {
    name: "Dev Tools",
    description: "Inspect local development tools and UI examples.",
    href: "/dev",
    icon: PlaygroundIcon,
  },
];

export const defaultNavigationSections: NavigationSection[] = [
  { id: "chats", items: defaultChatsNavigation },
  { id: "overview", items: defaultOverviewNavigation },
  { label: "Build", items: defaultBuildNavigation },
  { label: "Activity", items: defaultActivityNavigation },
  { label: "Blocks", items: defaultBlocksNavigation },
  { label: "Quality", items: defaultQualityNavigation },
  { items: defaultBottomNavigation },
  { label: "System", items: defaultDurableNavigation, defaultCollapsed: true },
  { label: "Dev", items: defaultDevNavigation, devOnly: true },
];

/** True when `pathname` is `href` or sits beneath it, on a segment boundary. */
function isUnder(pathname: string, href: string): boolean {
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * The label of the sidebar group that owns `pathname`, or `undefined` when the
 * page sits outside a labelled group — Chats, Overview and Settings have no group header,
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
  return bestNavigationMatch(pathname, sections)?.section.label;
}

/**
 * The destination that owns `pathname` (same longest-href rule as the group), so
 * other surfaces, such as in-app links in chat replies, can show its sidebar icon.
 */
export function navigationItemForPath(
  pathname: string,
  sections: NavigationSection[] = defaultNavigationSections,
): NavigationItem | undefined {
  return bestNavigationMatch(pathname, sections)?.item;
}

function bestNavigationMatch(
  pathname: string,
  sections: NavigationSection[],
): { section: NavigationSection; item: NavigationItem } | undefined {
  let best: { length: number; section: NavigationSection; item: NavigationItem } | undefined;
  for (const section of sections) {
    for (const item of section.items) {
      const href = item.activePrefix ?? item.href;
      if (!isUnder(pathname, href)) continue;
      if (!best || href.length > best.length) {
        best = { length: href.length, section, item };
      }
    }
  }
  return best;
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

/** The System section: platform users only, like the pages it links to. */
export function isDurableNavigationSection(section: NavigationSection): boolean {
  return section.items.some(
    (item) => isUnder(item.href, "/durable") || isUnder(item.href, "/system"),
  );
}
