"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useEffect } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Plus } from "lucide-react";
import {
  isTriggerChannel,
  useAgentChannels,
  usePublishAgentChannel,
} from "@/hooks/use-agent-channels";
import { usePolicies } from "@/hooks/use-policies";
import { buttonVariants } from "@/components/ui/button";
import { ChannelList } from "@/components/agents/channels/channel-list";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
import { BudgetPanel } from "@/components/budgets/budget-panel";
import { PageColumns, PageMain, PageRail, RailSection } from "@/components/layout";
import type { Agent } from "@/lib/api/types";
import { isReadOnlyStatus } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";

export function AgentIntegrationsPanel({
  agent,
  initialChannelId = null,
}: {
  agent: Agent;
  /** Channel to open on arrival, from `?channel=`. */
  initialChannelId?: string | null;
}) {
  const ui = useUiTranslations();
  const { channels, isLoading } = useAgentChannels(agent.id);
  const { can: canAgent } = usePolicies("agents");
  const { can: canBudget } = usePolicies("budgets");
  const publishEndpoint = usePublishAgentChannel(agent.id);
  const readOnly = isReadOnlyStatus(agent.status);
  const canManage = canAgent("agent.manage") && !readOnly;
  const canDangerous = canAgent("agent.dangerous") && !readOnly;
  const canViewBudgets = canBudget("budget.view");
  const canManageBudgets = canBudget("budget.manage");

  const doors = channels.filter(({ channel }) => !isTriggerChannel(channel));
  const publishedDoors = doors.filter(
    ({ channel }) => channel.enabled && channel.status === "live",
  ).length;
  // Schedules created as channels before schedules became triggers are listed
  // with the triggers and open on their channel page.
  const scheduleChannels = channels
    .filter(({ channel }) => isTriggerChannel(channel))
    .map(({ channel }) => channel);
  // Channels and schedules open on their own page; an old `?channel=` link
  // lands there.
  const router = useRouter();
  const linkedChannelId = channels.some(({ channel }) => channel.id === initialChannelId)
    ? initialChannelId
    : null;
  useEffect(() => {
    if (linkedChannelId) router.replace(`/agents/${agent.id}/channels/${linkedChannelId}`);
  }, [agent.id, linkedChannelId, router]);

  return (
    <div className="mx-auto flex w-full max-w-[1600px] flex-col gap-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">{ui("Channels and triggers")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {ui("How people and software reach this agent, and when it starts on its own.")}
        </p>
      </div>
      <PageColumns>
        <PageMain className="gap-6">
          <section className="flex flex-col gap-3">
            <div className="flex flex-wrap items-end justify-between gap-3">
              <div className="min-w-0 flex-[1_1_20rem]">
                <h3 className="text-lg font-semibold tracking-tight">{ui("Channels")}</h3>
                <p className="text-sm text-muted-foreground">
                  {doors.length === 0
                    ? ui("Where people and software reach this agent.")
                    : ui(
                        "Where people and software reach this agent. {value0}, {value1} published.",
                        {
                          value0: `${doors.length} ${pluralize(doors.length, "channel")}`,
                          value1: publishedDoors,
                        },
                      )}
                </p>
              </div>
              {canManage && (
                <Link
                  href={`/agents/${agent.id}/channels/new`}
                  className={buttonVariants({ variant: "outline" })}
                >
                  <Plus className="size-4" />
                  {ui("Add channel")}
                </Link>
              )}
            </div>

            {isLoading ? (
              <p className="text-sm text-muted-foreground">{ui("Loading channels…")}</p>
            ) : doors.length === 0 ? (
              <div className="flex flex-col items-center border bg-card px-6 py-10 text-center">
                <h4 className="text-base font-semibold">{ui("Nobody can reach this agent yet")}</h4>
                <p className="mt-1.5 max-w-md text-sm text-muted-foreground">
                  {ui("Add a channel to give people a chat, or give your code an address to call.")}
                </p>
                {canManage && (
                  <div className="mt-5 flex flex-wrap justify-center gap-2">
                    <Link
                      href={`/agents/${agent.id}/channels/new?kind=work_chat`}
                      className={buttonVariants({ variant: "outline" })}
                    >
                      {ui("Add Work Chat")}
                    </Link>
                    <Link
                      href={`/agents/${agent.id}/channels/new?kind=api`}
                      className={buttonVariants({ variant: "outline" })}
                    >
                      {ui("Add Agent API")}
                    </Link>
                    <Link
                      href={`/agents/${agent.id}/channels/new`}
                      className={buttonVariants({ variant: "ghost" })}
                    >
                      {ui("See all channel types")}
                    </Link>
                  </div>
                )}
              </div>
            ) : (
              <ChannelList
                channels={doors.map(({ channel }) => channel)}
                hrefFor={(channel) => `/agents/${agent.id}/channels/${channel.id}`}
                onPublishChange={
                  canDangerous
                    ? (channel, publish) =>
                        publishEndpoint.mutate({ channelId: channel.id, publish })
                    : undefined
                }
                publishPending={publishEndpoint.isPending}
              />
            )}
          </section>

          <AgentTriggersPanel
            agentId={agent.id}
            scheduleChannels={scheduleChannels}
            readOnly={readOnly}
          />
        </PageMain>

        <PageRail>
          {canViewBudgets && (
            <RailSection label={ui("Costs and usage policies")}>
              <BudgetPanel subjectType="agent" subjectId={agent.id} canManage={canManageBudgets} />
            </RailSection>
          )}
        </PageRail>
      </PageColumns>
    </div>
  );
}
