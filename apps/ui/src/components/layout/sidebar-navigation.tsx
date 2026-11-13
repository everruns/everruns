/**
 * Decisions:
 * - Navigation rendering stays data-driven so product forks can replace sections without forking layout chrome.
 * - Collapsible section state is local; no URL persistence needed for sidebar affordances.
 */
"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Component, useState, type ReactNode } from "react";
import Link, { useLinkStatus } from "next/link";
import { useRouter } from "next/navigation";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { FeatureFlags } from "@/lib/api/types";
import { ExperimentalBadge } from "@/components/ui/experimental-badge";
import { WarningBadge } from "@/components/ui/warning-badge";
import type { NavigationItem, NavigationSection } from "./sidebar";
import { visibleNavigationSections } from "@/lib/navigation";
import { useNavigationPolicy } from "@/hooks/use-navigation-policy";
import { useOrg } from "@/providers/org-provider";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";
import { useHomeAttention } from "@/components/home/use-home-attention";

const isDev = process.env.NODE_ENV === "development";

function NavigationPending() {
  const ui = useUiTranslations();
  const { pending } = useLinkStatus();
  return pending ? (
    <span role="status" className="ml-auto">
      <Loader2 aria-hidden="true" className="size-3.5 animate-spin" />
      <span className="sr-only">{ui("Loading page")}</span>
    </span>
  ) : null;
}

/**
 * The count renders on every page, so a failure in any of its queries must not
 * take the page down with it: it just disappears. Overview shows the real error.
 */
class QuietBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? null : this.props.children;
  }
}

/** How many items wait on Overview's "Needs attention" list; hidden at zero. */
function OverviewCount({ className }: { className?: string }) {
  const ui = useUiTranslations();
  const { items } = useHomeAttention();
  if (items.length === 0) return null;
  return (
    <span
      className={cn(
        "min-w-5 bg-primary/10 px-1.5 text-center text-[11px] font-semibold tabular-nums text-primary",
        className ?? "ml-auto",
      )}
      aria-label={ui("{value0} need attention", { value0: items.length })}
    >
      {items.length}
    </span>
  );
}

/** Overview's attention count, isolated so a failing query only hides it. */
export function OverviewCountBadge({ className }: { className?: string }) {
  return (
    <QuietBoundary>
      <OverviewCount className={className} />
    </QuietBoundary>
  );
}

function NavLink({ item, pathname }: { item: NavigationItem; pathname: string }) {
  const ui = useUiTranslations();
  const router = useRouter();
  const chatMode = useChatMode();

  const activePath = item.activePrefix ?? item.href;
  // Main chat opens beside the current page; on a Chat route it is a plain link.
  const onChatRoute = pathname === "/chats" || pathname.startsWith("/chats/");
  const opensBeside = item.href === "/chats" && !!chatMode?.openBeside && !onChatRoute;
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
      onClick={
        opensBeside
          ? (event) => {
              if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;
              event.preventDefault();
              chatMode?.openBeside?.();
            }
          : undefined
      }
      className={cn(
        // The active item tracks the route commit. A color transition keeps
        // painting after the page is already on screen, which reads as the
        // sidebar redrawing once the page has loaded.
        // Chat stays a primary destination (semibold, always readable) but
        // shares this row: an inset card shifted its label right and made
        // the hover feel like a different control.
        "flex items-center gap-2.5 border-l-2 px-3 py-1.5 text-[13px] leading-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
        isActive
          ? "border-l-primary bg-primary/5 font-semibold text-foreground"
          : "border-l-transparent hover:border-l-border hover:bg-card/80",
        !isActive &&
          (item.prominent
            ? "font-semibold text-foreground"
            : "font-medium text-muted-foreground hover:text-foreground"),
      )}
    >
      <item.icon className="icon-sharp h-4 w-4 shrink-0" />
      {ui(item.name)}
      {item.href === "/chats" && chatMode && (
        <kbd className="ml-auto text-[10px] text-muted-foreground" title={ui("⌘⇧J: full screen")}>
          {ui("⌘J")}
        </kbd>
      )}
      {item.href === "/home" && (
        <QuietBoundary>
          <OverviewCount />
        </QuietBoundary>
      )}
      {item.warningTooltip && <WarningBadge tooltip={item.warningTooltip} />}
      {item.experimental && !item.warningTooltip && <ExperimentalBadge />}
      <NavigationPending />
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
  // A collapsed-by-default group still opens when the current page is in it,
  // so the active link is never hidden.
  const ui = useUiTranslations();
  const [collapsed, setCollapsed] = useState(
    (section.defaultCollapsed ?? false) &&
      !section.items.some((item) => pathname === item.href || pathname.startsWith(`${item.href}/`)),
  );

  const isCollapsible = section.defaultCollapsed !== undefined && section.label;

  return (
    <>
      {!isFirst && <div className="my-3" />}
      {section.label &&
        (isCollapsible ? (
          <button
            type="button"
            onClick={() => setCollapsed((value) => !value)}
            className="flex w-full items-center justify-between px-3 py-0.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
          >
            {ui(section.label)}
            {collapsed ? (
              <ChevronRight className="h-3.5 w-3.5" />
            ) : (
              <ChevronDown className="h-3.5 w-3.5" />
            )}
          </button>
        ) : (
          <p className="px-3 py-0.5 text-xs font-medium text-muted-foreground">
            {ui(section.label)}
          </p>
        ))}
      {!collapsed &&
        section.items.map((item) => (
          <NavLink key={ui(item.name)} item={item} pathname={pathname} />
        ))}
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
  const canNavigate = useNavigationPolicy();
  const visibleSections = visibleNavigationSections(
    sections,
    featureFlags,
    hasRole,
    isDev,
    canNavigate,
  );

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
