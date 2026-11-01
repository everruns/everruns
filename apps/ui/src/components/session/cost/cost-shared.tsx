// Shared pieces of the session Cost tab. See knowledge/ui/session-cost.md.
//
// Decision: category colors are chart series, so they use raw hues (allowed by
// the Slate foundations for series). Both sets were run through the dataviz
// palette validator: light against the white card, dark against the dark card.

import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export type CostCategoryKey = "uncached_input" | "cache_read" | "cache_write" | "output";

export const CATEGORY_ORDER: CostCategoryKey[] = [
  "uncached_input",
  "cache_read",
  "cache_write",
  "output",
];

/** Background classes per category, light then dark. */
export const CATEGORY_BG: Record<CostCategoryKey | "unattributed", string> = {
  uncached_input: "bg-[#2c4f8f] dark:bg-[#5b86d6]",
  cache_read: "bg-[#0a9396] dark:bg-[#17a598]",
  cache_write: "bg-[#8a5cc4] dark:bg-[#a07ee0]",
  output: "bg-[#b07a12] dark:bg-[#b98a22]",
  unattributed: "bg-muted-foreground/50",
};

export function formatUsd(usd: number, locale: string): string {
  const digits = usd !== 0 && Math.abs(usd) < 0.01 ? 4 : 2;
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(usd);
}

export function formatCompact(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 2 }).format(
    value,
  );
}

export function formatShare(part: number, total: number, locale: string): string {
  if (total <= 0) return "—";
  const share = part / total;
  if (share > 0 && share < 0.001)
    return `<${new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 }).format(0.001)}`;
  return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 }).format(
    share,
  );
}

/** Seconds as "1h 12m", "4m 3s" or "12s". */
export function formatSeconds(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  if (m > 0) return s % 60 > 0 ? `${m}m ${s % 60}s` : `${m}m`;
  return `${s}s`;
}

export function CostCard({
  title,
  meta,
  actions,
  children,
  className,
}: {
  title: string;
  meta?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("border border-border bg-card", className)}>
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-4 py-3">
        <h2 className="text-[15px] font-semibold">{title}</h2>
        <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
          {meta}
          {actions}
        </div>
      </header>
      {children}
    </section>
  );
}

export function ShareBar({ share, className }: { share: number; className: string }) {
  const width = Math.max(0, Math.min(100, share * 100));
  return (
    <span className="inline-block h-1.5 w-24 bg-muted align-middle" aria-hidden>
      <span className={cn("block h-1.5", className)} style={{ width: `${width}%` }} />
    </span>
  );
}

export function Swatch({ className }: { className: string }) {
  return <span className={cn("inline-block h-2.5 w-2.5 shrink-0", className)} aria-hidden />;
}
