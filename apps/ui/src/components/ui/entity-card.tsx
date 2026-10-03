"use client";

import * as React from "react";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { cn } from "@/lib/utils";

/**
 * Shared card for entity list/grid views (agents, harnesses, skills, providers,
 * sessions, examples, dashboard rows). It standardizes the parts that should feel
 * the same everywhere — the leading icon, the (optionally linked) title with a
 * copy button, the right-aligned header actions, and the click affordance — while
 * leaving the body content fully up to the caller via `children`.
 *
 * The card itself is inert. Its title and explicit relationship/action links
 * navigate independently so nested actions stay unambiguous.
 */
export interface EntityCardProps {
  /** Layout: vertical grid card (default) or horizontal list row. */
  variant?: "grid" | "row";
  /** Leading icon or avatar rendered before the title. */
  icon?: React.ReactNode;
  /** Title content. */
  title: React.ReactNode;
  /** When set, the title becomes a link to this href. */
  href?: string;
  /** Extra classes for the title — e.g. entity lifecycle styling. */
  titleClassName?: string;
  /** When set, renders a copy button next to the title. */
  copyValue?: string;
  /** Secondary line under the title (grid) — e.g. mono id/name or description. */
  subtitle?: React.ReactNode;
  /** Content rendered inline after the title (row) — e.g. status/reference badges. */
  inlineBadges?: React.ReactNode;
  /**
   * Right-aligned header content. In the grid variant this is the status badge
   * area; in the row variant this is the trailing actions column.
   */
  headerActions?: React.ReactNode;
  /** Body content. */
  children?: React.ReactNode;
  /** Footer content (grid only). Use {@link EntityCardFooter} for the common split. */
  footer?: React.ReactNode;
  className?: string;
}

function EntityCardTitleLink({
  href,
  className,
  children,
}: {
  href?: string;
  className?: string;
  children: React.ReactNode;
}) {
  if (href) {
    return (
      <Link href={href} className={cn("hover:underline", className)}>
        {children}
      </Link>
    );
  }
  if (className) {
    return <span className={className}>{children}</span>;
  }
  return <>{children}</>;
}

export function EntityCard({
  variant = "grid",
  icon,
  title,
  href,
  titleClassName,
  copyValue,
  subtitle,
  inlineBadges,
  headerActions,
  children,
  footer,
  className,
}: EntityCardProps) {
  if (variant === "row") {
    return (
      <div
        className={cn(
          "group flex items-start justify-between border bg-card p-4 transition-colors hover:border-muted-foreground/40",
          className,
        )}
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          {icon && <div className="mt-0.5 flex-shrink-0">{icon}</div>}
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              {copyValue ? (
                <EntityIdentity
                  value={copyValue}
                  labelClassName={cn("font-medium", titleClassName)}
                >
                  <EntityCardTitleLink href={href}>{title}</EntityCardTitleLink>
                </EntityIdentity>
              ) : (
                <EntityCardTitleLink
                  href={href}
                  className={cn("truncate font-medium", titleClassName)}
                >
                  {title}
                </EntityCardTitleLink>
              )}
              {inlineBadges}
            </div>
            {children}
          </div>
        </div>
        {headerActions && (
          <div className="ml-2 flex flex-shrink-0 items-center gap-2">{headerActions}</div>
        )}
      </div>
    );
  }

  return (
    <Card
      className={cn(
        "gap-0 bg-card transition-colors hover:border-muted-foreground/40",
        footer && "pb-0",
        className,
      )}
    >
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3 space-y-0 pb-3">
        <div data-slot="entity-card-heading" className="flex min-w-0 flex-1 items-start gap-3">
          {icon && <div className="flex-shrink-0">{icon}</div>}
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <div className="flex min-w-0 items-center gap-2">
              <CardTitle className="min-w-0 flex-1 text-lg leading-snug">
                {copyValue ? (
                  <EntityIdentity value={copyValue} labelClassName={titleClassName}>
                    <EntityCardTitleLink href={href}>{title}</EntityCardTitleLink>
                  </EntityIdentity>
                ) : (
                  <EntityCardTitleLink href={href} className={cn("block truncate", titleClassName)}>
                    {title}
                  </EntityCardTitleLink>
                )}
              </CardTitle>
            </div>
            {subtitle && <div className="min-w-0">{subtitle}</div>}
          </div>
        </div>
        {headerActions && (
          <div className="flex flex-shrink-0 items-center gap-1">{headerActions}</div>
        )}
      </CardHeader>
      <CardContent className="flex-1">{children}</CardContent>
      {footer && <div className="mt-3">{footer}</div>}
    </Card>
  );
}

/**
 * Common grid-card footer: muted metadata on the left, actions on the right.
 */
export function EntityCardFooter({
  meta,
  actions,
  className,
}: {
  meta?: React.ReactNode;
  actions?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-wrap items-center gap-2 border-t border-border/60 px-4 py-2",
        className,
      )}
    >
      {meta && <div className="min-w-0 text-xs text-muted-foreground">{meta}</div>}
      {actions && <div className="ml-auto">{actions}</div>}
    </div>
  );
}

/** Quiet tags shared by entity cards, distinct from capability chips. */
export function EntityCardTags({ tags }: { tags: string[] }) {
  const [expanded, setExpanded] = React.useState(false);
  if (tags.length === 0) return null;
  const visibleTags = expanded ? tags : tags.slice(0, 3);
  return (
    <div
      aria-label="Tags"
      className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground"
    >
      {visibleTags.map((tag) => (
        <span key={tag} className="max-w-40 truncate bg-muted/60 px-2 py-1" title={tag}>
          {tag}
        </span>
      ))}
      {tags.length > 3 && (
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={expanded ? "Show fewer tags" : `Show ${tags.length - 3} more tags`}
          onClick={() => setExpanded(!expanded)}
          className="bg-muted/60 px-2 py-1 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
        >
          {expanded ? "Less" : `+${tags.length - 3}`}
        </button>
      )}
    </div>
  );
}

/** Keep large capability sets readable without losing configuration details. */
export function EntityCardCapabilities({
  children,
  label,
  ariaLabel,
}: {
  children: React.ReactNode;
  label?: string;
  ariaLabel?: string;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const items = React.Children.toArray(children);
  if (items.length === 0 && !label) return null;
  return (
    <div
      aria-label={ariaLabel ?? label ?? "Capabilities"}
      className="flex flex-wrap items-center gap-x-3 gap-y-2"
    >
      {label && <span className="text-[11px] text-muted-foreground">{label}</span>}
      {expanded ? items : items.slice(0, 4)}
      {items.length > 4 && (
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={
            expanded ? "Show fewer capabilities" : `Show ${items.length - 4} more capabilities`
          }
          onClick={() => setExpanded(!expanded)}
          className="bg-muted/60 px-2 py-1 text-xs text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
        >
          {expanded ? "Less" : `+${items.length - 4}`}
        </button>
      )}
    </div>
  );
}

/** Descriptions carry more contrast than secondary configuration and metadata. */
export function EntityCardDescription({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("mb-3 line-clamp-2 text-sm leading-relaxed text-foreground/75", className)}>
      {children}
    </div>
  );
}

/** Compact, reusable relationship/detail row for entity card bodies. */
export function EntityCardDetail({
  icon,
  label,
  children,
  className,
}: {
  icon?: React.ReactNode;
  label: React.ReactNode;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      data-slot="entity-card-detail"
      className={cn("flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground", className)}
    >
      {icon && <span className="inline-flex shrink-0 items-center">{icon}</span>}
      <span className="shrink-0">{label}</span>
      <span className="flex min-w-0 items-center gap-1.5">{children}</span>
    </div>
  );
}
