"use client";

import Link from "next/link";
import {
  CalendarClock,
  ChevronDown,
  Hash,
  Monitor,
  MoreHorizontal,
  Play,
  Webhook,
} from "lucide-react";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { CronLabel } from "@/components/apps/cron-label";
import { MiniTimeline, type TimelineBin } from "@/components/apps/mini-timeline";
import type {
  AgUiChannelConfig,
  App,
  AppChannel,
  ChannelType,
  ScheduleChannelConfig,
  SlackChannelConfig,
  WebhookChannelConfig,
} from "@/lib/api/types";
import { getChannelTypeDisplayName } from "@/lib/app-channels";

function iconFor(kind: ChannelType) {
  switch (kind) {
    case "schedule":
      return CalendarClock;
    case "webhook":
      return Webhook;
    case "ag_ui":
      return Monitor;
    case "fcp":
      return Hash;
    case "slack":
      return Slack;
    default:
      return Hash;
  }
}

function relativeTime(value?: string | null): string {
  if (!value) return "never";
  const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 60) return formatter.format(seconds, "second");
  if (abs < 3600) return formatter.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return formatter.format(Math.round(seconds / 3600), "hour");
  return formatter.format(Math.round(seconds / 86400), "day");
}

function channelName(channel: AppChannel): string {
  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return config.message?.trim() || "Scheduled invocation";
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return config.channel_id || config.team_id || "Slack channel";
  }
  if (channel.channel_type === "webhook") return "Webhook endpoint";
  if (channel.channel_type === "ag_ui") return "AG-UI endpoint";
  if (channel.channel_type === "fcp") return "FCP endpoint";
  return getChannelTypeDisplayName(channel.channel_type);
}

function channelSubline(channel: AppChannel, app: App): React.ReactNode {
  const statusText = !channel.enabled
    ? "Paused"
    : app.status === "published"
      ? "Active"
      : "Active when published";

  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return (
      <>
        Schedule · <CronLabel expr={config.cron_expression} tz={config.timezone} /> · {statusText}
      </>
    );
  }
  const lastInvokedAt = channel.last_invoked_at ?? null;
  return (
    <>
      {getChannelTypeDisplayName(channel.channel_type)} · {relativeTime(lastInvokedAt)} ·{" "}
      {channel.enabled ? "Enabled" : "Disabled"}
    </>
  );
}

function detailText(channel: AppChannel): string {
  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return `Session mode: ${config.session_mode ?? "shared_session"}`;
  }
  if (channel.channel_type === "webhook") {
    const config = channel.channel_config as WebhookChannelConfig;
    return `Token: ${config.token_configured || config.token ? "configured" : "not configured"}`;
  }
  if (channel.channel_type === "ag_ui") {
    const config = channel.channel_config as AgUiChannelConfig;
    return `Tool visibility: ${config.tool_visibility ?? "generic"}`;
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return `Session strategy: ${config.session_strategy ?? "per_thread"}`;
  }
  return "Configured channel";
}

export function ChannelRow({
  channel,
  app,
  expanded,
  onToggle,
  onRunNow,
  configureHref,
  timeline = [],
}: {
  channel: AppChannel;
  app: App;
  expanded: boolean;
  onToggle: () => void;
  onRunNow?: () => void;
  configureHref: string;
  timeline?: TimelineBin[];
}) {
  const Icon = iconFor(channel.channel_type);
  const canRunNow =
    !!onRunNow &&
    channel.channel_type === "schedule" &&
    channel.enabled &&
    app.status === "published";

  return (
    <div className="border bg-card">
      <div className="grid gap-3 p-4 md:grid-cols-[minmax(0,1fr)_140px_120px_120px_48px] md:items-center">
        <button
          type="button"
          onClick={onToggle}
          className="min-w-0 text-left"
          aria-label={`Toggle ${channelName(channel)} details`}
        >
          <div className="flex min-w-0 items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center border bg-background">
              <Icon className="size-4" />
            </span>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <p className="truncate font-medium">{channelName(channel)}</p>
                <Badge variant="outline">{getChannelTypeDisplayName(channel.channel_type)}</Badge>
                <Badge variant={channel.enabled ? "default" : "secondary"}>
                  {channel.enabled ? "active" : "disabled"}
                </Badge>
              </div>
              <p className="mt-1 text-sm text-muted-foreground">{channelSubline(channel, app)}</p>
            </div>
          </div>
        </button>
        <div>
          <p className="text-xs font-medium uppercase text-muted-foreground">
            {channel.channel_type === "schedule" ? "Next run" : "Last invoke"}
          </p>
          <p className="mt-1 text-sm">
            {channel.channel_type === "schedule"
              ? relativeTime(channel.next_run_at ?? null)
              : relativeTime(channel.last_invoked_at ?? null)}
          </p>
        </div>
        <div>
          <p className="text-xs font-medium uppercase text-muted-foreground">Runs · 24h</p>
          <p className="mt-1 text-sm">0</p>
        </div>
        <MiniTimeline runs={timeline} length={12} className="hidden md:flex" />
        <DropdownMenu>
          <DropdownMenuTrigger
            className={buttonVariants({ variant: "ghost", size: "icon" })}
            aria-label="Channel actions"
          >
            <MoreHorizontal className="size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuPositioner>
            <DropdownMenuContent>
              <DropdownMenuItem render={<Link href={configureHref} />}>Configure</DropdownMenuItem>
              <DropdownMenuItem onClick={onRunNow} disabled={!canRunNow}>
                <Play className="size-4" />
                Run now
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenuPositioner>
        </DropdownMenu>
      </div>
      {expanded && (
        <div className="border-t bg-muted/20 px-4 py-3">
          <div className="grid gap-3 text-sm md:grid-cols-3">
            <div>
              <p className="text-xs font-medium uppercase text-muted-foreground">Configuration</p>
              <p className="mt-1">{detailText(channel)}</p>
            </div>
            <div>
              <p className="text-xs font-medium uppercase text-muted-foreground">Created</p>
              <p className="mt-1">{new Date(channel.created_at).toLocaleString()}</p>
            </div>
            <div className="flex items-center justify-between gap-3 md:justify-end">
              <Button type="button" variant="outline" size="sm" onClick={onToggle}>
                <ChevronDown className="size-4 rotate-180 transition-transform" />
                Collapse
              </Button>
              <Link href={configureHref} className={buttonVariants({ size: "sm" })}>
                Configure
              </Link>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
