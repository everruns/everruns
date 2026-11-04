"use client";

import Link from "next/link";
import { useState } from "react";
import { Check } from "lucide-react";
import {
  AgentIcon,
  HealthIcon,
  SandboxIcon,
  SessionIcon,
  WorkerIcon,
} from "@/components/icons/facet-icons";
import { buttonVariants } from "@/components/ui/button";
import { formatRelativeTime } from "@/lib/formatting";
import type { HomeAttentionItem, HomeSource } from "@/lib/org-home";
import { cn } from "@/lib/utils";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";

const SOURCE_ICON: Record<HomeSource, typeof AgentIcon> = {
  agents: AgentIcon,
  health: HealthIcon,
  sessions: SessionIcon,
  sandboxes: SandboxIcon,
  system: WorkerIcon,
};

const SOURCE_LABEL: Record<HomeSource, string> = {
  agents: "Agents",
  health: "Health",
  sessions: "Sessions",
  sandboxes: "Sandboxes",
  system: "System",
};

/** Rows shown before "Show all": enough to see what is blocking, short enough to scan. */
export const ATTENTION_PREVIEW = 7;

/**
 * Needs attention, loud when something is wrong and one quiet line when not.
 * Unlike the Agents page summary it starts open: on Home it is the point.
 */
export function HomeAttention({
  items,
  loading,
}: {
  items: HomeAttentionItem[];
  loading: boolean;
}) {
  const ui = useUiTranslations();
  const [all, setAll] = useState(false);

  if (items.length === 0) {
    return (
      <section
        aria-label={ui("Needs attention")}
        className="flex items-center gap-3 border bg-card px-4 py-3 text-sm"
        data-testid="home-attention-clear"
      >
        <span className="inline-flex size-6 shrink-0 items-center justify-center bg-success/10 text-success">
          <Check className="size-3.5" aria-hidden="true" />
        </span>
        <span className="font-semibold">
          {loading ? ui("Checking what needs attention…") : ui("Nothing needs attention.")}
        </span>
        {!loading && (
          <span className="text-muted-foreground">
            {ui("Agents, channels, sessions, sandboxes and limits all look fine.")}
          </span>
        )}
      </section>
    );
  }

  const shown = all ? items : items.slice(0, ATTENTION_PREVIEW);
  return (
    <section
      aria-label={ui("Needs attention")}
      className="border bg-card"
      data-testid="home-attention"
    >
      <header className="flex items-center gap-2.5 border-b px-4 py-3">
        <h2 className="text-sm font-semibold">{ui("Needs attention")}</h2>
        <span className="inline-flex h-5 min-w-5 items-center justify-center bg-destructive px-1 text-[11px] font-semibold text-destructive-foreground">
          {items.length}
        </span>
        <span className="hidden text-xs text-muted-foreground @md/home:inline">
          {ui("Blocking first, then the oldest")}
        </span>
        {items.length > ATTENTION_PREVIEW && (
          <button
            type="button"
            onClick={() => setAll((value) => !value)}
            className="ml-auto text-xs underline underline-offset-4"
          >
            {all ? ui("Show fewer") : ui("Show all {count}", { count: items.length })}
          </button>
        )}
      </header>
      <ul className="divide-y divide-border/60">
        {shown.map((item) => (
          <AttentionRow key={item.key} item={item} />
        ))}
      </ul>
    </section>
  );
}

function AttentionRow({ item }: { item: HomeAttentionItem }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const Icon = SOURCE_ICON[item.source];
  return (
    <li className="relative flex items-center gap-3.5 py-3 pr-4 pl-4" data-severity={item.severity}>
      <span
        aria-hidden="true"
        className={cn(
          "absolute inset-y-0 left-0 w-[3px]",
          item.severity === "error" && "bg-destructive",
          item.severity === "warning" && "bg-warning",
          item.severity === "info" && "bg-info/70",
        )}
      />
      <span
        className={cn(
          "hidden size-8 shrink-0 items-center justify-center border bg-background @md/home:inline-flex",
          item.severity === "error" && "text-destructive",
          item.severity === "warning" && "text-warning",
        )}
      >
        <Icon className="size-4" aria-hidden="true" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm leading-snug font-semibold">
          <span className="mr-2 text-[11px] font-medium text-muted-foreground">
            {ui(SOURCE_LABEL[item.source])}
          </span>
          {ui(item.title, item.values)}
        </p>
        <p className="mt-0.5 text-[13px] text-foreground/70">
          {item.subject && <span className="font-medium text-foreground">{item.subject} · </span>}
          {ui(item.body, item.values)}
        </p>
      </div>
      {item.since && (
        <span className="hidden w-24 shrink-0 text-right text-xs text-muted-foreground @2xl/home:inline">
          {formatRelativeTime(item.since, displayLocale)}
        </span>
      )}
      <Link
        href={item.href}
        className={cn(buttonVariants({ variant: "outline", size: "sm" }), "shrink-0")}
      >
        {ui(item.action)}
      </Link>
    </li>
  );
}
