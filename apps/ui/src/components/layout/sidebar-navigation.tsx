/**
 * Decisions:
 * - Navigation rendering stays data-driven so product forks can replace sections without forking layout chrome.
 * - Collapsible section state is local; no URL persistence needed for sidebar affordances.
 */
"use client";

import { useState, type ReactNode } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import type { FeatureFlags } from "@/lib/api/types";
import { ExperimentalBadge } from "@/components/ui/experimental-badge";
import { WarningBadge } from "@/components/ui/warning-badge";
import type { NavigationItem, NavigationSection } from "./sidebar";
import { visibleNavigationSections } from "@/lib/navigation";
import { useOrg } from "@/providers/org-provider";

const isDev = process.env.NODE_ENV === "development";

function NavLink({ item, pathname }: { item: NavigationItem; pathname: string }) {
  const router = useRouter();

  const activePath = item.activePrefix ?? item.href;
  const isActive = item.exact
    ? pathname === activePath
    : pathname === activePath || pathname.startsWith(`${activePath}/`);

  return (
    <Link
      href={item.href}
      prefetch={false}
      aria-current={isActive ? "page" : undefined}
      onMouseEnter={item.prefetch === false ? undefined : () => router.prefetch(item.href)}
      onFocus={item.prefetch === false ? undefined : () => router.prefetch(item.href)}
      className={cn(
        // The active item tracks the route commit. A color transition keeps
        // painting after the page is already on screen, which reads as the
        // sidebar redrawing once the page has loaded.
        "flex items-center gap-2.5 text-[13px] font-medium leading-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
        item.prominent
          ? cn(
              // Chat is an always-available destination, not a label for its side conversations.
              "mx-2.5 mb-1 border-l-2 px-2.5 py-2 font-semibold text-foreground",
              isActive ? "border-l-accent bg-muted" : "border-l-transparent hover:bg-muted",
            )
          : cn(
              "border-l-2 px-3 py-1.5",
              isActive
                ? "border-l-primary bg-primary/5 font-semibold text-foreground"
                : "border-l-transparent text-muted-foreground hover:border-l-border hover:bg-card/80 hover:text-foreground",
            ),
      )}
    >
      <item.icon className="icon-sharp h-4 w-4 shrink-0 stroke-[2.15]" />
      {item.name}
      {item.warningTooltip && <WarningBadge tooltip={item.warningTooltip} />}
      {item.experimental && !item.warningTooltip && <ExperimentalBadge />}
    </Link>
  );
}

function NavSection({
  section,
  pathname,
  isFirst,
  renderExtra,
}: {
  section: NavigationSection;
  pathname: string;
  isFirst: boolean;
  renderExtra?: (section: NavigationSection) => ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(section.defaultCollapsed ?? false);

  const isCollapsible = section.defaultCollapsed !== undefined && section.label;

  return (
    <>
      {!isFirst && <div className="my-3" />}
      {section.label &&
        (isCollapsible ? (
          <button
            type="button"
            onClick={() => setCollapsed((value) => !value)}
            className="flex w-full items-center justify-between px-3 py-0.5 text-[10px] font-medium uppercase tracking-[0.12em] text-muted-foreground transition-colors hover:text-foreground"
          >
            {section.label}
            {collapsed ? (
              <ChevronRight className="h-3.5 w-3.5" />
            ) : (
              <ChevronDown className="h-3.5 w-3.5" />
            )}
          </button>
        ) : (
          <p className="px-3 py-0.5 text-[10px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
            {section.label}
          </p>
        ))}
      {!collapsed &&
        section.items.map((item) => <NavLink key={item.name} item={item} pathname={pathname} />)}
      {!collapsed && renderExtra?.(section)}
    </>
  );
}

export function SidebarNavigation({
  sections,
  pathname,
  featureFlags,
  renderSectionExtra,
}: {
  sections: NavigationSection[];
  pathname: string;
  featureFlags: FeatureFlags;
  /** Extra content appended inside a section, below its links. Used by the
   *  Chats section to hang the live thread list off the nav entry. */
  renderSectionExtra?: (section: NavigationSection) => ReactNode;
}) {
  const orgContext = useOrg();
  const hasRole = orgContext.hasRole ?? (() => true);
  const visibleSections = visibleNavigationSections(sections, featureFlags, hasRole, isDev);

  return (
    <nav className="flex-1 min-h-0 overflow-y-auto space-y-0.5 bg-background py-2.5">
      {visibleSections.map((section, index) => (
        <NavSection
          key={section.label ?? `section-${index}`}
          section={section}
          pathname={pathname}
          isFirst={index === 0}
          renderExtra={renderSectionExtra}
        />
      ))}
    </nav>
  );
}
