"use client";

import { useState } from "react";
import Link from "next/link";
import {
  ChevronDown,
  Cpu,
  Globe,
  ListChecks,
  PlugZap,
  ShieldAlert,
  TriangleAlert,
} from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { useHealthIssueAction } from "@/hooks/use-health-issues";
import { usePublishAgentChannel } from "@/hooks/use-agent-channels";
import { templateSetupPath } from "@/lib/agent-template-setup";
import type { AttentionItem, AttentionKind } from "@/lib/agents-home";
import { cn } from "@/lib/utils";

const KIND_ICON: Record<AttentionKind, typeof Cpu> = {
  model: Cpu,
  permission: ShieldAlert,
  connection: PlugZap,
  "public-access": Globe,
  setup: ListChecks,
};

/**
 * Needs attention, quiet by default: one summary line naming the affected
 * agents, collapsed until opened, and absent entirely when nothing is wrong.
 */
export function NeedsAttention({
  items,
  canManage,
}: {
  items: AttentionItem[];
  canManage: boolean;
}) {
  const [open, setOpen] = useState(false);
  if (items.length === 0) return null;

  const agentNames = Array.from(new Set(items.map((item) => item.agentName).filter(Boolean)));
  const hasError = items.some((item) => item.severity === "error");

  return (
    <section
      aria-label="Needs attention"
      className={cn(
        "border border-l-4 bg-card",
        hasError ? "border-l-destructive" : "border-l-warning",
      )}
    >
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex w-full min-w-0 items-center gap-3 px-4 py-3 text-left"
      >
        <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
        <span className="shrink-0 text-sm font-semibold">
          {items.length === 1 ? "1 agent setting needs" : `${items.length} agent settings need`}{" "}
          attention
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] text-muted-foreground">
          {agentNames.join(", ")}
        </span>
        <span className="inline-flex shrink-0 items-center gap-1 text-[13px] text-muted-foreground">
          {open ? "Hide" : "Show"}
          <ChevronDown
            className={cn("size-3.5 transition-transform", open && "rotate-180")}
            aria-hidden="true"
          />
        </span>
      </button>
      {open && (
        <ul className="divide-y border-t">
          {items.map((item) => (
            <AttentionRow key={item.key} item={item} canManage={canManage} />
          ))}
        </ul>
      )}
    </section>
  );
}

function AttentionRow({ item, canManage }: { item: AttentionItem; canManage: boolean }) {
  const Icon = KIND_ICON[item.kind];
  return (
    <li className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center">
      <span className="hidden size-8 shrink-0 items-center justify-center border sm:inline-flex">
        <Icon className="size-4" aria-hidden="true" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-baseline gap-x-2 text-sm">
          <span
            className={cn(
              "text-xs",
              item.severity === "error" ? "text-destructive" : "text-warning",
            )}
          >
            {item.label}
          </span>
          <span className="font-semibold">{item.title}</span>
          <Link href={`/agents/${item.agentId}`} className="underline underline-offset-2">
            {item.agentName}
          </Link>
        </p>
        <p className="mt-0.5 text-[13px] text-muted-foreground">
          {item.body}
          {item.detail && <span> · {item.detail}</span>}
        </p>
      </div>
      {canManage && <AttentionActions item={item} />}
    </li>
  );
}

function AttentionActions({ item }: { item: AttentionItem }) {
  const check = useHealthIssueAction("check");
  const snooze = useHealthIssueAction("snooze");
  const publish = usePublishAgentChannel(item.agentId);
  const channelHref = item.channelId
    ? `/agents/${item.agentId}/channels/${item.channelId}`
    : `/agents/${item.agentId}?tab=integrations`;
  const linkClass = buttonVariants({ variant: "outline", size: "sm" });

  switch (item.kind) {
    case "model":
      return (
        <div className="flex shrink-0 flex-wrap gap-2">
          <Link href={`/agents/${item.agentId}?mode=edit`} className={linkClass}>
            Change model
          </Link>
        </div>
      );
    case "permission":
    case "connection":
      return (
        <div className="flex shrink-0 flex-wrap gap-2">
          {item.healthIssueId && (
            <>
              <Button
                variant="outline"
                size="sm"
                disabled={check.isPending}
                onClick={() => check.mutate(item.healthIssueId!)}
              >
                Check again
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={snooze.isPending}
                onClick={() => snooze.mutate(item.healthIssueId!)}
              >
                Remind me tomorrow
              </Button>
            </>
          )}
          <Link href={channelHref} className={linkClass}>
            Fix channel
          </Link>
        </div>
      );
    case "setup":
      return (
        <div className="flex shrink-0 flex-wrap gap-2">
          <Link
            href={templateSetupPath(item.agentId, item.exampleName ?? "")}
            className={linkClass}
          >
            Finish setup
          </Link>
        </div>
      );
    case "public-access":
      return (
        <div className="flex shrink-0 flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={publish.isPending || !item.channelId}
            onClick={() => publish.mutate({ channelId: item.channelId!, publish: false })}
          >
            Unpublish
          </Button>
          <Link href={channelHref} className={linkClass}>
            Require sign-in
          </Link>
        </div>
      );
  }
}
