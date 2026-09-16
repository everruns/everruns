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
import { Switch } from "@/components/ui/switch";
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
  PublicChatChannelConfig,
  ScheduleChannelConfig,
  SlackChannelConfig,
  WebhookChannelConfig,
} from "@/lib/api/types";
import { getChannelTypeDisplayName, getEndpointLifecyclePresentation } from "@/lib/app-channels";

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
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    return config.branding?.display_name?.trim() || "Public Chat";
  }
  return getChannelTypeDisplayName(channel.channel_type);
}

function channelSubline(channel: AppChannel, _app: App): React.ReactNode {
  const { description } = getEndpointLifecyclePresentation(channel);

  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return (
      <>
        Schedule · <CronLabel expr={config.cron_expression} tz={config.timezone} /> · {description}
      </>
    );
  }
  const lastInvokedAt = channel.last_invoked_at ?? null;
  return (
    <>
      {getChannelTypeDisplayName(channel.channel_type)} · {relativeTime(lastInvokedAt)} ·{" "}
      {description}
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
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    const hasSignIn = !!config.auth && config.auth.mode !== "anonymous";
    const tokenProtected = !!config.token_configured || !!config.token;
    let access: string;
    if (hasSignIn) {
      access = "sign-in required";
    } else if (config.anonymous === false) {
      // anonymous off without a sign-in provider locks the endpoint.
      access = "no access configured";
    } else if (tokenProtected) {
      access = "token-protected";
    } else {
      access = "anonymous";
    }
    const captcha = config.captcha?.enabled ? " · Turnstile" : "";
    return `Access: ${access}${captcha}`;
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
  onPublishChange,
  publishPending = false,
  usePanel,
  configureHref,
  timeline = [],
}: {
  channel: AppChannel;
  app: App;
  expanded: boolean;
  onToggle: () => void;
  onRunNow?: () => void;
  /// Per-endpoint publish (EVE-1007). Omitted where the caller has no write
  /// path — the App detail page keeps its App-level switch.
  onPublishChange?: (publish: boolean) => void;
  publishPending?: boolean;
  /// "How do I call this" for this endpoint specifically. Rendered inside the
  /// expanded row rather than a separate tab, so the snippet can carry this
  /// endpoint's real URL instead of a placeholder.
  usePanel?: React.ReactNode;
  configureHref: string;
  timeline?: TimelineBin[];
}) {
  const Icon = iconFor(channel.channel_type);
  const lifecycle = getEndpointLifecyclePresentation(channel);
  const { isLive } = lifecycle;
  const canRunNow = !!onRunNow && channel.channel_type === "schedule" && isLive;
  const panelId = `endpoint-panel-${channel.id}`;

  return (
    <div className="border bg-card">
      <div
        className={
          // Four fixed columns plus a rail left roughly 130px for the name at
          // an ordinary 1280px window, which truncated "Webhook endpoint" to
          // "W." and stacked its badges. The metric columns — both of which
          // read 0 until run aggregation lands — are held back until there is
          // width for them; the name, status and actions are what the row is
          // for. The publish switch shares the actions cell for the same
          // reason.
          onPublishChange
            ? "grid gap-3 p-4 md:grid-cols-[minmax(0,1fr)_140px_88px] 2xl:grid-cols-[minmax(0,1fr)_140px_120px_120px_88px] md:items-center"
            : "grid gap-3 p-4 md:grid-cols-[minmax(0,1fr)_140px_48px] 2xl:grid-cols-[minmax(0,1fr)_140px_120px_120px_48px] md:items-center"
        }
      >
        <button
          type="button"
          onClick={onToggle}
          className="min-w-0 text-left"
          aria-expanded={expanded}
          aria-controls={panelId}
          aria-label={`${expanded ? "Collapse" : "Expand"} ${channelName(channel)} details`}
        >
          <div className="flex min-w-0 items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center border bg-background">
              <Icon className="size-4" />
            </span>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <p className="truncate font-medium">{channelName(channel)}</p>
                <Badge variant="outline">{getChannelTypeDisplayName(channel.channel_type)}</Badge>
                <Badge variant={isLive ? "default" : "secondary"}>{lifecycle.label}</Badge>
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
        <div className="hidden 2xl:block">
          <p className="text-xs font-medium uppercase text-muted-foreground">Runs · 24h</p>
          <p className="mt-1 text-sm">0</p>
        </div>
        <MiniTimeline runs={timeline} length={12} className="hidden 2xl:flex" />
        <div className="flex items-center justify-end gap-1">
          {onPublishChange && (
            <Switch
              checked={isLive}
              onCheckedChange={onPublishChange}
              disabled={publishPending || !channel.enabled}
              aria-label={`${isLive ? "Unpublish" : "Publish"} ${channelName(channel)}`}
            />
          )}
          <DropdownMenu>
            <DropdownMenuTrigger
              className={buttonVariants({ variant: "ghost", size: "icon" })}
              aria-label="Channel actions"
            >
              <MoreHorizontal className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner>
              <DropdownMenuContent>
                <DropdownMenuItem render={<Link href={configureHref} />}>
                  Configure
                </DropdownMenuItem>
                <DropdownMenuItem onClick={onRunNow} disabled={!canRunNow}>
                  <Play className="size-4" />
                  Run now
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        </div>
      </div>
      {expanded && (
        <div id={panelId} className="border-t bg-muted/20 px-4 py-3">
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
          {usePanel && <div className="mt-4 border-t pt-4">{usePanel}</div>}
        </div>
      )}
    </div>
  );
}
