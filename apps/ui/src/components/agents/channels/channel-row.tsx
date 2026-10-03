"use client";

import { useState } from "react";
import Link from "next/link";
import { ChevronDown, MoreHorizontal, Play } from "lucide-react";
import { slackConnectionLabel } from "@/components/agents/integrations/slack-setup-guidance";
import { ChannelIcon } from "./channel-icon";
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
  AgentChannel,
  PublicChatChannelConfig,
  ScheduleChannelConfig,
  SlackChannelConfig,
  WebhookChannelConfig,
} from "@/lib/api/types";
import { getChannelTypeDisplayName, getChannelLifecyclePresentation } from "@/lib/channel-display";

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

function channelName(channel: AgentChannel): string {
  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return config.message?.trim() || "Scheduled invocation";
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return config.channel_id ? `Slack · ${config.channel_id}` : "Slack channel";
  }
  if (channel.channel_type === "webhook") return "Webhook channel";
  if (channel.channel_type === "ag_ui") return "AG-UI channel";
  if (channel.channel_type === "fcp") return "FCP channel";
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    return config.branding?.display_name?.trim() || "Public Chat";
  }
  return getChannelTypeDisplayName(channel.channel_type);
}

function channelSubline(channel: AgentChannel): React.ReactNode {
  const { description } = getChannelLifecyclePresentation(channel);

  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return (
      <>
        Schedule · <CronLabel expr={config.cron_expression} tz={config.timezone} /> · {description}
      </>
    );
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return (
      <>
        {slackConnectionLabel(config)} · {config.team_id ?? "Workspace not selected"}
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

function detailText(channel: AgentChannel): string {
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
    const auth = channel.auth ?? config.auth;
    const hasSignIn = !!auth && auth.mode !== "anonymous";
    const tokenProtected = !!config.token_configured || !!config.token;
    let access: string;
    if (hasSignIn) {
      access = "sign-in required";
    } else if (config.anonymous === false) {
      // anonymous off without a sign-in provider locks the channel.
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
  expanded,
  onToggle,
  onRunNow,
  onPublishChange,
  publishPending = false,
  usePanel,
  configureHref,
  timeline = [],
}: {
  channel: AgentChannel;
  expanded: boolean;
  onToggle: () => void;
  onRunNow?: () => void;
  onPublishChange?: (publish: boolean) => void;
  publishPending?: boolean;
  /// "How do I call this" for this channel specifically. Rendered inside the
  /// expanded row rather than a separate tab, so the snippet can carry this
  /// channel's real URL instead of a placeholder.
  usePanel?: React.ReactNode;
  configureHref?: string;
  timeline?: TimelineBin[];
}) {
  const lifecycle = getChannelLifecyclePresentation(channel);
  const { isLive } = lifecycle;
  const canRunNow = !!onRunNow && channel.channel_type === "schedule" && isLive;
  const panelId = `channel-panel-${channel.id}`;
  const inlineConfiguration = channel.channel_type === "slack" && !!configureHref;
  const [hasExpanded, setHasExpanded] = useState(expanded);
  const toggle = () => {
    setHasExpanded(true);
    onToggle();
  };

  return (
    <div className="border bg-card">
      <div
        className={
          // Four fixed columns plus a rail left roughly 130px for the name at
          // an ordinary 1280px window, which truncated "Webhook channel" to
          // "W." and stacked its badges. The metric columns — both of which
          // read 0 until run aggregation lands — are held back until there is
          // width for them; the name, status and actions are what the row is
          // for. The publish switch shares the actions cell for the same
          // reason.
          onPublishChange
            ? "grid gap-3 p-4 md:grid-cols-[minmax(0,1fr)_140px_160px] 2xl:grid-cols-[minmax(0,1fr)_140px_120px_120px_160px] md:items-center"
            : "grid gap-3 p-4 md:grid-cols-[minmax(0,1fr)_140px_48px] 2xl:grid-cols-[minmax(0,1fr)_140px_120px_120px_48px] md:items-center"
        }
      >
        <button
          type="button"
          onClick={toggle}
          className="min-w-0 text-left hover:bg-muted/40 focus-visible:outline-2 focus-visible:outline-ring"
          aria-expanded={expanded}
          aria-controls={panelId}
          aria-label={`${expanded ? "Collapse" : "Expand"} ${channelName(channel)} details`}
        >
          <div className="flex min-w-0 items-start gap-3">
            <ChevronDown
              aria-hidden="true"
              className={`mt-2 size-4 shrink-0 text-muted-foreground transition-transform ${expanded ? "" : "-rotate-90"}`}
            />
            <span className="flex size-9 shrink-0 items-center justify-center border bg-background">
              <ChannelIcon kind={channel.channel_type} className="size-4" />
            </span>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <p className="truncate font-medium">{channelName(channel)}</p>
                <Badge variant="outline">{getChannelTypeDisplayName(channel.channel_type)}</Badge>
                <Badge variant={isLive ? "default" : "secondary"}>{lifecycle.label}</Badge>
              </div>
              <p className="mt-1 text-sm text-muted-foreground">{channelSubline(channel)}</p>
              <p className="mt-1 text-xs">
                {expanded ? "Hide" : "Show"} {inlineConfiguration ? "configuration" : "details"}
              </p>
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
            <label className="flex items-center gap-2 text-xs">
              {isLive ? "Published" : "Draft"}
              <Switch
                checked={isLive}
                onCheckedChange={onPublishChange}
                disabled={publishPending || !channel.enabled}
                aria-label={`${isLive ? "Unpublish" : "Publish"} ${channelName(channel)}`}
              />
            </label>
          )}
          {(configureHref || onRunNow) && (
            <DropdownMenu>
              <DropdownMenuTrigger
                className={buttonVariants({ variant: "ghost", size: "icon" })}
                aria-label="Channel actions"
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuPositioner>
                <DropdownMenuContent>
                  {configureHref && (
                    <DropdownMenuItem render={<Link href={configureHref} />}>
                      {inlineConfiguration ? "Endpoint options" : "Configure"}
                    </DropdownMenuItem>
                  )}
                  {onRunNow && (
                    <DropdownMenuItem onClick={onRunNow} disabled={!canRunNow}>
                      <Play className="size-4" />
                      Run now
                    </DropdownMenuItem>
                  )}
                </DropdownMenuContent>
              </DropdownMenuPositioner>
            </DropdownMenu>
          )}
        </div>
      </div>
      {(expanded || (inlineConfiguration && hasExpanded)) && (
        <div id={panelId} hidden={!expanded} className="border-t bg-muted/20 px-4 py-3">
          {!inlineConfiguration && (
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
                <Button type="button" variant="outline" size="sm" onClick={toggle}>
                  <ChevronDown className="size-4 rotate-180 transition-transform" />
                  Collapse
                </Button>
                {configureHref && (
                  <Link href={configureHref} className={buttonVariants({ size: "sm" })}>
                    Configure
                  </Link>
                )}
              </div>
            </div>
          )}
          {usePanel && (
            <div className={inlineConfiguration ? "" : "mt-4 border-t pt-4"}>{usePanel}</div>
          )}
        </div>
      )}
    </div>
  );
}
