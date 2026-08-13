// Navigation model — the single source of which group owns which route.
//
// The sidebar renders these sections, and `PageBreadcrumb` derives a page's
// group from the same table (EVE-869). Keeping the definitions here rather than
// in `sidebar.tsx` lets the breadcrumb read them without pulling the whole
// sidebar component tree into every page.
//
// Navigation is grouped by what you do with a thing, not what it is. The
// placement rule and the dismissed alternatives live in
// `knowledge/ui/information-architecture.md`; consult it before adding an
// entity to a group.

import {
  Boxes,
  Brain,
  Calendar,
  ChartColumn,
  ClipboardCheck,
  Cog,
  FlaskConical,
  Library,
  ListTodo,
  MessageCircle,
  MessageSquare,
  Rocket,
  Server,
  Settings,
  Shield,
  Telescope,
  UserRound,
  Workflow,
} from "lucide-react";
import type { IconComponent } from "@/lib/capability-icons";
import { registryNavigationItems } from "@/lib/registry-navigation";
import type { FeatureFlags } from "@/lib/api/types";

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

export const defaultChatsNavigation: NavigationItem[] = [
  { name: "Chats", href: "/chats", icon: MessageCircle },
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
 * without `/agent-identities` colliding with it, and `/durable/workers` picks
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
