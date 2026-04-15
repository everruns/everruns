/**
 * Decisions:
 * - Navigation rendering stays data-driven so product forks can replace sections without forking layout chrome.
 * - Collapsible section state is local; no URL persistence needed for sidebar affordances.
 */
"use client";

import { useState } from "react";
import Link from "next/link";
import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import type { FeatureFlags } from "@/lib/api/types";
import { ExperimentalBadge } from "@/components/ui/experimental-badge";
import { WarningBadge } from "@/components/ui/warning-badge";
import type { NavigationItem, NavigationSection } from "./sidebar";

const isDev = process.env.NODE_ENV === "development";

function NavLink({
  item,
  pathname,
  featureFlags,
}: {
  item: NavigationItem;
  pathname: string;
  featureFlags: FeatureFlags;
}) {
  if (item.flag && !featureFlags[item.flag]) return null;

  const isActive = item.exact
    ? pathname === item.href
    : pathname === item.href || pathname.startsWith(`${item.href}/`);

  return (
    <Link
      href={item.href}
      className={cn(
        "flex items-center gap-2.5 border-l-2 px-3 py-1.5 text-[13px] font-semibold leading-5 transition-colors",
        isActive
          ? "border-l-primary bg-card text-foreground"
          : "border-l-transparent text-muted-foreground hover:border-l-border hover:bg-card/80 hover:text-foreground",
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
  featureFlags,
  isFirst,
}: {
  section: NavigationSection;
  pathname: string;
  featureFlags: FeatureFlags;
  isFirst: boolean;
}) {
  const [collapsed, setCollapsed] = useState(section.defaultCollapsed ?? false);
  if (section.devOnly && !isDev) return null;

  const isCollapsible = section.defaultCollapsed !== undefined && section.label;

  return (
    <>
      {!isFirst && <div className="my-2 border-t" />}
      {section.label &&
        (isCollapsible ? (
          <button
            type="button"
            onClick={() => setCollapsed((value) => !value)}
            className="flex w-full items-center justify-between px-3 py-0.5 text-[10px] font-medium uppercase tracking-[0.18em] text-muted-foreground transition-colors hover:text-foreground"
          >
            {section.label}
            {collapsed ? (
              <ChevronRight className="h-3.5 w-3.5" />
            ) : (
              <ChevronDown className="h-3.5 w-3.5" />
            )}
          </button>
        ) : (
          <p className="px-3 py-0.5 text-[10px] font-medium uppercase tracking-[0.18em] text-muted-foreground">
            {section.label}
          </p>
        ))}
      {!collapsed &&
        section.items.map((item) => (
          <NavLink key={item.name} item={item} pathname={pathname} featureFlags={featureFlags} />
        ))}
    </>
  );
}

export function SidebarNavigation({
  sections,
  pathname,
  featureFlags,
}: {
  sections: NavigationSection[];
  pathname: string;
  featureFlags: FeatureFlags;
}) {
  let visibleIndex = 0;

  return (
    <nav className="flex-1 min-h-0 overflow-y-auto space-y-0.5 bg-background py-2.5">
      {sections.map((section, index) => {
        if (section.devOnly && !isDev) return null;
        const isFirst = visibleIndex === 0;
        visibleIndex += 1;
        return (
          <NavSection
            key={section.label ?? `section-${index}`}
            section={section}
            pathname={pathname}
            featureFlags={featureFlags}
            isFirst={isFirst}
          />
        );
      })}
    </nav>
  );
}
