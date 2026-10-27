"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { Pause } from "lucide-react";
import { ChannelIcon } from "@/components/agents/channels/channel-icon";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { getChannelLifecyclePresentation, getChannelTypeDisplayName } from "@/lib/channel-display";
import type { Agent, AgentChannelSummary } from "@/lib/api/types";
import { cn } from "@/lib/utils";

function channelLabel(channel: AgentChannelSummary) {
  switch (channel.channel_type) {
    case "a2a":
      return "A2A";
    case "fcp":
      return "FCP";
    case "public_chat":
      return "Public chat";
    case "api":
      return "Agent API";
    default:
      return getChannelTypeDisplayName(channel.channel_type);
  }
}

function presentation(agent: Agent, channel: AgentChannelSummary) {
  if (agent.status !== "active") return { label: "disabled", description: "Agent unavailable" };
  if (agent.exposures_suspended) return { label: "disabled", description: "Suspended" };
  return getChannelLifecyclePresentation(channel);
}

export function AgentCardChannels({
  agent,
  canManage = false,
}: {
  agent: Agent;
  canManage?: boolean;
}) {
  const ui = useUiTranslations();
  const channels = agent.channels?.filter((channel) => channel.channel_type !== "schedule");
  const integrationsHref = `/agents/${agent.id}?tab=integrations`;
  const visibility = [
    "",
    "hidden @min-[21rem]/card:inline-flex",
    "hidden @min-[28rem]/card:inline-flex",
  ];
  const overflowVisibility = [
    "@min-[21rem]/card:hidden",
    "hidden @min-[21rem]/card:inline-flex @min-[28rem]/card:hidden",
    "hidden @min-[28rem]/card:inline-flex",
  ];

  return (
    <div
      aria-label={ui("Channels")}
      className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2 text-xs"
    >
      <span className="shrink-0 text-muted-foreground">{ui("Channels")}</span>
      {channels === undefined ? (
        <span className="text-muted-foreground">{ui("Not loaded")}</span>
      ) : channels.length === 0 ? (
        <>
          <span className="text-muted-foreground">{ui("None configured")}</span>
          {canManage && agent.status === "active" && (
            <Link href={`/agents/${agent.id}/channels/new`} className="ml-auto hover:underline">
              {ui("Add")}
            </Link>
          )}
        </>
      ) : (
        <TooltipProvider>
          <div className="flex min-w-0 items-center gap-1.5">
            {channels.slice(0, 3).map((channel, index) => {
              const state = presentation(agent, channel);
              const label = channelLabel(channel);
              return (
                <Tooltip key={channel.id}>
                  <TooltipTrigger asChild>
                    <Link
                      href={integrationsHref}
                      aria-label={ui("{value0}: {value1}", {
                        value0: label,
                        value1: state.description,
                      })}
                      className={cn(
                        "inline-flex shrink-0 items-center gap-1.5 text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring",
                        visibility[index],
                      )}
                    >
                      <ChannelIcon kind={channel.channel_type} className="icon-sharp size-3.5" />
                      <span>{label}</span>
                      {state.label === "disabled" ? (
                        <Pause className="size-3 text-muted-foreground" aria-hidden="true" />
                      ) : (
                        <span
                          aria-hidden="true"
                          className={cn(
                            "size-2 rounded-full",
                            state.label === "live" ? "bg-success" : "border border-warning",
                          )}
                        />
                      )}
                    </Link>
                  </TooltipTrigger>
                  <TooltipContent>
                    {getChannelTypeDisplayName(channel.channel_type)} · {state.description}
                  </TooltipContent>
                </Tooltip>
              );
            })}
            {[1, 2, 3].map(
              (visible) =>
                channels.length > visible && (
                  <Link
                    key={visible}
                    href={integrationsHref}
                    aria-label={ui("View all {value0} channels", { value0: channels.length })}
                    className={cn(
                      "inline-flex shrink-0 items-center bg-muted/60 px-2 py-1 text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring",
                      overflowVisibility[visible - 1],
                    )}
                  >
                    +{channels.length - visible}
                  </Link>
                ),
            )}
          </div>
        </TooltipProvider>
      )}
    </div>
  );
}
