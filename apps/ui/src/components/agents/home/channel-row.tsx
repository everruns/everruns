"use client";

import Link from "next/link";
import { ArrowUpRight } from "lucide-react";
import { ChannelIcon } from "@/components/agents/channels/channel-icon";
import { Button } from "@/components/ui/button";
import { usePublishAgentChannel } from "@/hooks/use-agent-channels";
import type { OrgExposure } from "@/hooks/use-org-exposures";
import { RunBars } from "./run-bars";
import { StateDot } from "./state-dot";
import {
  agentChannelHref,
  channelContext,
  channelTrafficLine,
  channelShortName,
  channelState,
  channelStateLabel,
  initials,
} from "@/lib/agents-home";
import { channelPublishControl } from "@/lib/channel-display";
import type { ChannelActivity } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { formatRelativeTime, pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";

const EMPTY_DAYS = Array.from({ length: 7 }, () => 0);

/**
 * One channel: where it points, which agent answers, a week of traffic, its
 * state and access, and the one switch that matters (publish or unpublish).
 * Public live channels get a tint, because they are the rows someone scans for.
 */
export function ChannelRow({
  exposure,
  activity,
  canManage,
}: {
  exposure: OrgExposure;
  activity: ChannelActivity | undefined;
  canManage: boolean;
}) {
  const { channel, agent } = exposure;
  const state = channelState(exposure);
  const context = channelContext(channel);
  const sessions = activity?.sessions ?? 0;
  const daily = activity?.daily ?? EMPTY_DAYS;
  const agentName = agent ? getDisplayName(agent) : "Unassigned";
  const lastSession = activity?.last_session_at ?? exposure.lastInvokedAt;

  return (
    <li
      className={cn(
        "grid gap-x-5 gap-y-2 px-4 py-3 text-[13px] lg:grid-cols-[minmax(0,1.6fr)_minmax(0,1.2fr)_8.5rem_7rem_8rem_5.5rem_6rem] lg:items-center",
        exposure.publiclyReachable && "bg-destructive/5",
      )}
    >
      <div className="flex min-w-0 items-center gap-3">
        <ChannelIcon kind={channel.channel_type} className="size-4 shrink-0" />
        <div className="min-w-0">
          <p className="truncate">
            {agent ? (
              <Link
                href={agentChannelHref(agent.id, channel.id)}
                className="inline-flex max-w-full items-center gap-1 font-medium hover:underline"
              >
                {channelShortName(channel.channel_type)}
                <ArrowUpRight
                  className="size-3 shrink-0 text-muted-foreground"
                  aria-hidden="true"
                />
              </Link>
            ) : (
              <span className="font-medium">{channelShortName(channel.channel_type)}</span>
            )}
            {context && <span className="text-muted-foreground"> {context}</span>}
          </p>
          <p className="truncate font-mono text-xs text-muted-foreground">{channel.id}</p>
        </div>
      </div>

      <div className="min-w-0">
        {agent ? (
          <Link
            href={`/agents/${agent.id}?tab=integrations`}
            className="inline-flex max-w-full items-center gap-2 hover:underline"
          >
            <span className="inline-flex size-6 shrink-0 items-center justify-center bg-accent/25 text-[10px] font-semibold">
              {initials(agentName)}
            </span>
            <span className="truncate font-medium">{agentName}</span>
            <ArrowUpRight className="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
          </Link>
        ) : (
          <span className="text-muted-foreground">{agentName}</span>
        )}
      </div>

      <div className="min-w-0">
        <RunBars
          buckets={daily.map((count) => ({ runs: count, failed: 0 }))}
          label={`${sessions} ${pluralize(sessions, "session")} in the last 7 days`}
          className="h-4"
        />
        <p className="mt-1 text-xs text-muted-foreground">{channelTrafficLine(activity)}</p>
      </div>

      <p className="flex items-center gap-2">
        <StateDot
          tone={
            state === "live"
              ? "live"
              : state === "paused"
                ? "paused"
                : state === "draft"
                  ? "draft"
                  : "muted"
          }
        />
        {channelStateLabel(state)}
      </p>

      <p
        className={cn(
          exposure.anonymous
            ? exposure.publiclyReachable
              ? "text-destructive"
              : "text-muted-foreground"
            : "text-muted-foreground",
        )}
      >
        {exposure.anonymous ? "Public access" : "Sign-in required"}
      </p>

      <p className="text-muted-foreground">
        {lastSession ? formatRelativeTime(lastSession) : "None in 7d"}
      </p>

      <div className="lg:text-right">
        {canManage && agent && (state === "live" || state === "draft") && (
          <PublishButton agentId={agent.id} channelId={channel.id} live={state === "live"} />
        )}
      </div>
    </li>
  );
}

function PublishButton({
  agentId,
  channelId,
  live,
}: {
  agentId: string;
  channelId: string;
  live: boolean;
}) {
  const publish = usePublishAgentChannel(agentId);
  const hint = channelPublishControl({
    enabled: true,
    status: live ? "live" : "draft",
  }).hint;
  return (
    <Button
      variant="outline"
      size="sm"
      disabled={publish.isPending}
      title={hint}
      onClick={() => publish.mutate({ channelId, publish: !live })}
    >
      {live ? "Unpublish" : "Publish"}
    </Button>
  );
}
