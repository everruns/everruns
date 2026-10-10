"use client";

import Link from "next/link";
import { CalendarClock, Webhook, Zap } from "lucide-react";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import { ChannelIcon } from "@/components/agents/channels/channel-icon";
import { GithubIcon } from "@/components/icons/github-icon";
import { RunBars } from "./run-bars";
import { StateDot, type DotTone } from "./state-dot";
import {
  agentChannelHref,
  agentNow,
  channelShortName,
  initials,
  type AttentionItem,
} from "@/lib/agents-home";
import { getChannelLifecyclePresentation } from "@/lib/channel-display";
import { getDisplayName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import type {
  Agent,
  AgentActivity,
  AgentChannelSummary,
  AgentTriggerSummary,
} from "@/lib/api/types";
import { cn } from "@/lib/utils";

const TRIGGER_ICON: Record<string, typeof Zap> = {
  schedule: CalendarClock,
  webhook: Webhook,
  github: GithubIcon as unknown as typeof Zap,
};

function triggerLabel(type: string): string {
  switch (type) {
    case "schedule":
      return "Schedule";
    case "webhook":
      return "Webhook trigger";
    case "github":
      return "GitHub";
    case "mcp_event":
      return "MCP events";
    default:
      return type;
  }
}

function channelTone(agent: Agent, channel: AgentChannelSummary): { tone: DotTone; text: string } {
  if (isArchivedStatus(agent.status)) return { tone: "muted", text: "Agent archived" };
  if (agent.exposures_suspended) return { tone: "paused", text: "Paused" };
  const lifecycle = getChannelLifecyclePresentation(channel);
  if (lifecycle.isLive) return { tone: "live", text: "Live" };
  if (lifecycle.label === "disabled") return { tone: "paused", text: "Paused" };
  return { tone: "draft", text: "Draft, not accepting traffic" };
}

const EMPTY_BUCKETS = Array.from({ length: 24 }, () => ({ runs: 0, failed: 0 }));

/**
 * One agent: who it is, what it is doing now, how it is reached, and how the
 * last 24 hours went. The name links to the agent page. Each channel links
 * to that channel on the agent.
 */
export function AgentRow({
  agent,
  activity,
  attention,
}: {
  agent: Agent;
  activity: AgentActivity | undefined;
  attention: AttentionItem[];
}) {
  const name = getDisplayName(agent);
  const now = agentNow(agent, activity);
  const channels = (agent.channels ?? []).filter((channel) => channel.channel_type !== "schedule");
  const triggers: AgentTriggerSummary[] = activity?.triggers ?? [];
  const firstIssue = attention[0];
  const runs = activity?.runs ?? 0;
  const failed = activity?.failed ?? 0;

  return (
    <li className="grid gap-x-6 gap-y-3 px-4 py-4 lg:grid-cols-[minmax(0,2.2fr)_minmax(0,1.3fr)_minmax(0,1.6fr)_9rem] lg:items-center">
      <div className="flex min-w-0 items-center gap-3">
        <AgentAvatar
          avatar={agent.avatar}
          size={36}
          fallback={
            <span className="inline-flex size-9 shrink-0 items-center justify-center bg-accent/25 text-xs font-semibold">
              {initials(name)}
            </span>
          }
        />
        <div className="min-w-0">
          <Link href={`/agents/${agent.id}`} className="font-semibold hover:underline">
            {name}
          </Link>
          <p className="truncate text-[13px] text-muted-foreground">
            <span className="font-mono text-xs">{agent.name}</span>
            {agent.description && <span> · {agent.description}</span>}
          </p>
        </div>
      </div>

      <div className="min-w-0 text-[13px]">
        <p className="flex items-center gap-2">
          <StateDot
            tone={now.tone === "running" ? "live" : now.tone === "paused" ? "paused" : "muted"}
          />
          <span className="font-medium">{now.label}</span>
        </p>
        {firstIssue && (
          <p
            className={cn(
              "mt-1 pl-4",
              firstIssue.severity === "error" ? "text-destructive" : "text-warning",
            )}
          >
            {firstIssue.title}
            {attention.length > 1 && ` (+${attention.length - 1} more)`}
          </p>
        )}
      </div>

      <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1.5 text-[13px]">
        {channels.length === 0 && triggers.length === 0 ? (
          <span className="text-muted-foreground">No channels or triggers</span>
        ) : (
          <>
            {channels.map((channel) => {
              const state = channelTone(agent, channel);
              const label = channelShortName(channel.channel_type);
              return (
                <Link
                  key={channel.id}
                  href={agentChannelHref(agent.id, channel.id)}
                  className="inline-flex items-center gap-1.5 hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                  title={`${label} · ${state.text}`}
                  aria-label={`${label} channel, ${state.text}`}
                >
                  <ChannelIcon kind={channel.channel_type} className="size-3.5" />
                  {label}
                  <StateDot tone={state.tone} />
                </Link>
              );
            })}
            {triggers.map((trigger, index) => {
              const Icon = TRIGGER_ICON[trigger.trigger_type] ?? Zap;
              const label = triggerLabel(trigger.trigger_type);
              const text = trigger.enabled ? "On" : "Off";
              return (
                <span
                  // Triggers have no id in this summary; order is stable (oldest first).
                  key={`${trigger.trigger_type}-${index}`}
                  className="inline-flex items-center gap-1.5"
                  title={`${label} trigger · ${text}`}
                >
                  <Icon className="size-3.5" aria-hidden="true" />
                  {label}
                  <StateDot tone={trigger.enabled ? "live" : "draft"} label={text} />
                </span>
              );
            })}
          </>
        )}
      </div>

      <div className="min-w-0">
        <RunBars
          buckets={activity?.hourly ?? EMPTY_BUCKETS}
          label={`${runs} ${pluralize(runs, "run")} in the last 24 hours, ${failed} failed`}
        />
        <p className="mt-1 text-xs text-muted-foreground">
          {runs === 0 ? (
            "No runs"
          ) : (
            <>
              {runs} {pluralize(runs, "run")}
              {failed > 0 && <span className="text-destructive"> · {failed} failed</span>}
            </>
          )}
        </p>
      </div>
    </li>
  );
}
