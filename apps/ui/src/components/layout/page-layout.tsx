"use client";

// Unified page layout — the single five-zone skeleton every standard page is built on.
//
// Zones (numbered ①–⑤ in the design source):
//   1. Breadcrumb     — PageBreadcrumb: the context line (root → name → mode).
//   2. Header         — PageMasthead: icon tile · title · status badge · meta, with a
//                       right-aligned action cluster that moves above the title when compact.
//   3. Control strip   — the shape-shifter: filters on list, stats on view, section nav
//                       on edit. Same slot, always. Compose with PageControlStrip plus
//                       SectionTabs / StatGrid as needed.
//   4. Body            — PageColumns: a primary column (PageMain) plus a fixed right rail
//                       (PageRail) for context.
//   5. Footer          — PageFooter: a quiet back link / count line that closes the page.
//
// Only the contents of each zone change per page; position and mechanics stay identical.
// These primitives are presentational and prop-driven so any page can adopt the language.

import { Fragment, type ReactNode } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { navigationGroupForPath } from "@/lib/navigation";
import { cn } from "@/lib/utils";

/* ─────────────────────────────── Container ─────────────────────────────── */

interface PageContainerProps {
  children: ReactNode;
  /** Opt into full-bleed width for dense tool pages. Defaults to a centered container. */
  fullWidth?: boolean;
  className?: string;
}

/**
 * Outer page wrapper. Provides padding and the consistent vertical rhythm between
 * the five zones so callers never hand-roll spacing.
 */
export function PageContainer({ children, fullWidth = false, className }: PageContainerProps) {
  return (
    <div
      className={cn(
        fullWidth ? "w-full" : "container mx-auto",
        "flex min-w-0 max-w-full flex-col gap-5 p-4 sm:gap-6 sm:p-6",
        className,
      )}
    >
      {children}
    </div>
  );
}

/* ────────────────────────────── 1 · Breadcrumb ─────────────────────────── */

export interface BreadcrumbItem {
  label: ReactNode;
  href?: string;
}

/**
 * Zone ① — the context line. Renders `Group → root → name → mode`; the final item
 * reads as the current location and is never linked.
 *
 * The leading group is derived from the sidebar's section table rather than
 * passed in, so a page's group is never stated in two places that can disagree
 * and moving a route between groups needs no page-level change (EVE-869). It is
 * a plain label, not a link — there is no group index page to point at. Pages
 * outside a labelled group (Chats, Settings) get no prefix.
 *
 * Pass `group={false}` for a page that renders a breadcrumb for somewhere other
 * than its own location, such as a component gallery.
 */
export function PageBreadcrumb({
  items,
  className,
  group = true,
}: {
  items: BreadcrumbItem[];
  className?: string;
  group?: boolean;
}) {
  const pathname = usePathname();
  const groupLabel = group ? navigationGroupForPath(pathname ?? "") : undefined;
  if (groupLabel) {
    items = [{ label: groupLabel }, ...items];
  }
  return (
    <nav
      aria-label="Breadcrumb"
      className={cn(
        "flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground",
        className,
      )}
    >
      {items.map((item, index) => {
        const isLast = index === items.length - 1;
        return (
          <Fragment key={index}>
            {index > 0 && (
              <span aria-hidden className="text-muted-foreground/50">
                /
              </span>
            )}
            {item.href && !isLast ? (
              <Link href={item.href} className="transition-colors hover:text-foreground">
                {item.label}
              </Link>
            ) : (
              <span className={isLast ? "text-foreground" : undefined}>{item.label}</span>
            )}
          </Fragment>
        );
      })}
    </nav>
  );
}

/* ──────────────────────────────── Icon tile ────────────────────────────── */

/**
 * The gold-tinted square that leads the header (and entity cards). `lg` is the
 * masthead size; `md` matches inline card rows.
 */
export function IconTile({
  icon,
  size = "lg",
  className,
}: {
  icon: ReactNode;
  size?: "md" | "lg";
  className?: string;
}) {
  return (
    <span
      data-slot="icon-tile"
      className={cn(
        "inline-flex flex-none items-center justify-center border bg-accent/20 text-foreground",
        size === "lg" ? "size-11 [&_svg]:size-[22px]" : "size-8 [&_svg]:size-4",
        className,
      )}
    >
      {icon}
    </span>
  );
}

/* ──────────────────────────────── 2 · Header ───────────────────────────── */

interface PageMastheadProps {
  /** Leading icon, wrapped in an IconTile automatically. */
  icon?: ReactNode;
  title: ReactNode;
  /** Exact API-facing entity ID copied beside the title. */
  entityId?: string;
  /** Status badges / counts shown inline after the title. */
  badges?: ReactNode;
  /** One-line description under the title. */
  description?: ReactNode;
  /** Meta run — small key/value spans rendered below the description. */
  meta?: ReactNode;
  /** Full right-aligned action cluster shown when the masthead has enough room. */
  actions?: ReactNode;
  /** Prioritized actions shown above the title instead of `actions` in constrained mastheads. */
  compactActions?: ReactNode;
  /** Frequent secondary actions shown on a separate row at tablet-like widths. */
  compactActionStrip?: ReactNode;
  className?: string;
}

/**
 * Zone ② — header anatomy shared by every page: gold icon tile · title · status
 * badge · meta run, with a right-aligned action cluster above the title when compact.
 */
export function PageMasthead({
  icon,
  title,
  entityId,
  badges,
  description,
  meta,
  actions,
  compactActions,
  compactActionStrip,
  className,
}: PageMastheadProps) {
  return (
    <div
      data-slot="page-masthead"
      className={cn(
        "@container/masthead flex flex-wrap items-start gap-4 border-b pb-4",
        className,
      )}
    >
      <div
        data-slot="page-masthead-identity"
        className={cn(
          "grid min-w-0 flex-[1_1_20rem] items-start gap-4",
          icon ? "grid-cols-[auto_minmax(0,1fr)]" : "grid-cols-1",
        )}
      >
        {icon && (
          <IconTile
            icon={icon}
            className={compactActions ? "sm:row-start-2 @5xl/masthead:row-start-1" : undefined}
          />
        )}
        {compactActions && (
          <div
            data-slot="page-masthead-compact-actions"
            className={cn(
              "row-start-1 ml-auto flex max-w-full flex-none flex-wrap items-center justify-end gap-2 sm:col-span-full @5xl/masthead:hidden [&>a]:max-w-full [&_[data-slot=button]]:h-auto [&_[data-slot=button]]:min-h-8 [&_[data-slot=button]]:max-w-full [&_[data-slot=button]]:whitespace-normal",
              icon ? "col-start-2 sm:col-start-1" : "col-start-1",
            )}
          >
            {compactActions}
          </div>
        )}
        <div
          className={cn(
            "min-w-0",
            icon && !compactActions && "col-start-2",
            compactActions &&
              "col-span-full row-start-2 sm:col-span-1 sm:row-start-2 @5xl/masthead:row-start-1",
            compactActions && icon && "sm:col-start-2",
          )}
        >
          <div className="flex min-w-0 flex-wrap items-center gap-2.5">
            <h1 className="min-w-0 text-2xl font-semibold tracking-tight text-foreground">
              {entityId ? (
                <EntityIdentity value={entityId} truncate={false}>
                  {title}
                </EntityIdentity>
              ) : (
                title
              )}
            </h1>
            {badges}
          </div>
          {description && (
            <p className="mt-1.5 break-words text-sm text-muted-foreground">{description}</p>
          )}
          {meta && (
            <div className="mt-2 flex min-w-0 flex-wrap gap-x-4 gap-y-1 text-[13px] text-muted-foreground [&>*]:min-w-0 [&>*]:break-words">
              {meta}
            </div>
          )}
        </div>
      </div>
      {actions && (
        <div
          data-slot="page-masthead-actions"
          className={cn(
            "ml-auto flex max-w-full flex-none flex-wrap items-center justify-end gap-2 [&>a]:max-w-full [&_[data-slot=button]]:h-auto [&_[data-slot=button]]:min-h-8 [&_[data-slot=button]]:max-w-full [&_[data-slot=button]]:whitespace-normal",
            compactActions && "hidden @5xl/masthead:flex",
          )}
        >
          {actions}
        </div>
      )}
      {compactActionStrip && (
        <div
          data-slot="page-masthead-compact-action-strip"
          className="hidden w-full flex-wrap items-center gap-2 @sm/masthead:flex @5xl/masthead:hidden [&>a]:max-w-full [&_[data-slot=button]]:h-auto [&_[data-slot=button]]:min-h-8 [&_[data-slot=button]]:max-w-full [&_[data-slot=button]]:whitespace-normal"
        >
          {compactActionStrip}
        </div>
      )}
    </div>
  );
}

/* ─────────────────────────────── 3 · Control strip ─────────────────────── */

/**
 * Zone ③ — the shape-shifter slot. A thin wrapper so the zone keeps its position
 * regardless of what it holds (filters, stats, or section nav).
 */
export function PageControlStrip({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={cn("min-w-0 max-w-full", className)}>{children}</div>;
}

export interface SectionTabItem {
  value: string;
  label: ReactNode;
  icon?: ReactNode;
}

/**
 * Underline section navigation used for the list status filter and the edit
 * section nav. Controlled — pair with local state and conditional panels.
 */
export function SectionTabs({
  value,
  onValueChange,
  items,
  className,
}: {
  value: string;
  onValueChange: (value: string) => void;
  items: SectionTabItem[];
  className?: string;
}) {
  return (
    <div
      className={cn("flex max-w-full items-center overflow-x-auto border-b", className)}
      role="tablist"
    >
      {items.map((item) => {
        const isActive = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={isActive}
            onClick={() => onValueChange(item.value)}
            className={cn(
              "-mb-px inline-flex shrink-0 items-center gap-2 border-b-2 px-3.5 py-2 text-[13px] font-medium transition-colors",
              isActive
                ? "border-primary text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            {item.icon}
            {item.label}
          </button>
        );
      })}
    </div>
  );
}

export interface PageJumpNavItem {
  href: `#${string}`;
  label: ReactNode;
}

/** Compact in-page navigation for long create and edit forms. */
export function PageJumpNav({
  items,
  className,
}: {
  items: PageJumpNavItem[];
  className?: string;
}) {
  return (
    <nav
      aria-label="Jump to section"
      className={cn(
        "flex max-w-full items-center gap-3 overflow-x-auto py-2 text-[13px] text-muted-foreground",
        className,
      )}
    >
      <span className="inline-flex shrink-0 items-center gap-1.5">
        <ArrowRight className="size-3.5" />
        Jump to
      </span>
      {items.map((item) => (
        <a
          key={item.href}
          href={item.href}
          className="shrink-0 transition-colors hover:text-foreground"
        >
          {item.label}
        </a>
      ))}
    </nav>
  );
}

/**
 * A single metric tile for the view control strip. Pass `value` for the big
 * number, `hint` for the caption, or `children` for custom content (e.g. a chart).
 */
export function StatCard({
  label,
  value,
  hint,
  children,
  className,
}: {
  label: ReactNode;
  value?: ReactNode;
  hint?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col gap-1.5 border bg-card p-4", className)}>
      <span className="font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
        {label}
      </span>
      {value !== undefined && (
        <span className="text-2xl font-semibold tracking-tight text-foreground">{value}</span>
      )}
      {children}
      {hint && <span className="text-[13px] text-muted-foreground">{hint}</span>}
    </div>
  );
}

/** Responsive grid for {@link StatCard}s. Defaults to four columns on wide screens. */
export function StatGrid({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("grid grid-cols-2 gap-3 lg:grid-cols-4", className)}>{children}</div>;
}

/* ──────────────────────────────── Empty state ──────────────────────────── */

interface EmptyStateProps {
  /** Leading glyph, rendered muted and centered above the title. */
  icon?: ReactNode;
  /** Headline — the primary message (e.g. "No agents yet"). */
  title: ReactNode;
  /** Optional supporting copy under the title. */
  description?: ReactNode;
  /** Optional call-to-action cluster, usually one primary button. */
  action?: ReactNode;
  className?: string;
}

/**
 * The shared empty/zero state for list bodies: a dashed panel with an optional
 * muted icon, a title, optional supporting copy, and an optional action. Every
 * list page uses this so empty states read identically across the app.
 */
export function EmptyState({ icon, title, description, action, className }: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center border border-dashed px-6 py-12 text-center",
        className,
      )}
    >
      {icon && (
        <span className="mb-4 inline-flex text-muted-foreground [&_svg]:size-10">{icon}</span>
      )}
      <h3 className="text-base font-semibold text-foreground">{title}</h3>
      {description && (
        <p className="mt-1.5 max-w-md text-sm text-muted-foreground">{description}</p>
      )}
      {action && <div className="mt-5 flex items-center justify-center gap-2">{action}</div>}
    </div>
  );
}

/* ──────────────────────────────── 4 · Body ─────────────────────────────── */

/**
 * Zone ④ — the two-column body: a flexible primary column plus a fixed-width
 * right rail. On narrow screens the rail stacks below.
 */
export function PageColumns({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn("grid grid-cols-1 gap-6 xl:grid-cols-[minmax(0,1fr)_320px]", className)}>
      {children}
    </div>
  );
}

/** The primary column inside {@link PageColumns}. */
export function PageMain({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn("@container/page-main flex min-w-0 flex-col gap-4", className)}>
      {children}
    </div>
  );
}

/** The fixed right rail inside {@link PageColumns}. */
export function PageRail({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex flex-col gap-4", className)}>{children}</div>;
}

/**
 * A rail facet card: small mono uppercase label over content. For richer titled
 * panels (e.g. "Recent sessions") use the standard Card component instead.
 */
export function RailSection({
  label,
  children,
  className,
}: {
  label: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("border bg-card p-4", className)}>
      <div className="mb-2.5 font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
        {label}
      </div>
      {children}
    </div>
  );
}

/* ──────────────────────────────── 5 · Footer ───────────────────────────── */

/**
 * Zone ⑤ — the closing line. A quiet bordered strip; pass a {@link BackLink} and
 * an optional count/secondary link.
 */
export function PageFooter({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "flex flex-wrap items-center justify-between gap-4 border-t pt-3.5 text-[13px] text-muted-foreground",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** Back link with a leading arrow, sized for the footer. */
export function BackLink({
  href,
  children,
  className,
}: {
  href: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Link
      href={href}
      className={cn(
        "inline-flex items-center gap-2 text-[13px] text-muted-foreground transition-colors hover:text-foreground",
        className,
      )}
    >
      <ArrowLeft className="size-4" />
      {children}
    </Link>
  );
}
