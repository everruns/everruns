"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useAgentSource } from "@/hooks/use-agent-source";
import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { PauseCircle, Plus } from "lucide-react";
import { useResumeAgentExposures, useSuspendAgentExposures } from "@/hooks/use-agents";
import {
  isTriggerChannel,
  useAgentChannels,
  usePublishAgentChannel,
  useTriggerAgentChannel,
} from "@/hooks/use-agent-channels";
import { useAgentTriggers } from "@/hooks/use-agent-triggers";
import { usePolicies } from "@/hooks/use-policies";
import { Switch } from "@/components/ui/switch";
import { buttonVariants } from "@/components/ui/button";
import { ChannelRow } from "@/components/agents/channels/channel-row";
import { ChannelList } from "@/components/agents/channels/channel-list";
import { AgentGitHubSection } from "@/components/agents/agent-github-card";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
import { BudgetPanel } from "@/components/budgets/budget-panel";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
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
  const { data: triggers = [] } = useAgentTriggers(agent.id);
  const { can: canAgent } = usePolicies("agents");
  const { can: canBudget } = usePolicies("budgets");
  const suspendExposures = useSuspendAgentExposures();
  const resumeExposures = useResumeAgentExposures();
  const publishEndpoint = usePublishAgentChannel(agent.id);
  const triggerEndpoint = useTriggerAgentChannel(agent.id);
  const [expandedId, setExpandedId] = useState<string | null>(initialChannelId);
  // A later visit with a different `?channel=` opens that row. Collapsing the
  // current one does not fight the address until the id changes.
  const [openedChannel, setOpenedChannel] = useState(initialChannelId);
  if (initialChannelId && initialChannelId !== openedChannel) {
    setOpenedChannel(initialChannelId);
    setExpandedId(initialChannelId);
  }

  const sourceQuery = useAgentSource(agent.id);
  const sourceOwns = (channel: { channel_config: unknown }) => {
    const name = (channel.channel_config as { package_name?: string })?.package_name;
    return (
      !!name &&
      (sourceQuery.isLoading ||
        sourceQuery.isError ||
        (sourceQuery.data?.mode !== "two_way" &&
          !!sourceQuery.data?.managed_channels.includes(name)))
    );
  };
  const suspended = agent.exposures_suspended ?? false;
  const canManage = canAgent("agent.manage") && !isReadOnlyStatus(agent.status);
  const canDangerous = canAgent("agent.dangerous") && !isReadOnlyStatus(agent.status);
  const canViewBudgets = canBudget("budget.view");
  const canManageBudgets = canBudget("budget.manage");

  const doors = channels.filter(({ channel }) => !isTriggerChannel(channel));
  const publishedDoors = doors.filter(
    ({ channel }) => channel.enabled && channel.status === "live",
  ).length;
  const scheduleEndpoints = channels.filter(({ channel }) => isTriggerChannel(channel));
  // Channels open on their own page now; an old `?channel=` link to one lands
  // there. Schedules still expand in place under Triggers.
  const router = useRouter();
  const linkedDoorId = doors.some(({ channel }) => channel.id === initialChannelId)
    ? initialChannelId
    : null;
  useEffect(() => {
    if (linkedDoorId) router.replace(`/agents/${agent.id}/channels/${linkedDoorId}`);
  }, [agent.id, linkedDoorId, router]);
  const githubTriggerCount = triggers.filter((trigger) => trigger.trigger_type === "github").length;

  return (
    <div className="mx-auto flex w-full max-w-[1600px] flex-col gap-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">{ui("Integrations")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {ui("Manage how this agent connects and runs.")}
        </p>
      </div>
      <PageColumns>
        <PageMain className="gap-6">
          <section className="flex flex-col gap-3">
            <div className="flex flex-wrap items-end justify-between gap-3">
              <div>
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
                  className={buttonVariants({ variant: "accent" })}
                >
                  <Plus className="size-4" />
                  {ui("Add channel")}
                </Link>
              )}
            </div>

            {suspended && doors.length > 0 && (
              <Notice icon={<PauseCircle className="size-4" />}>
                <NoticeTitle>{ui("All channels are paused")}</NoticeTitle>
                <NoticeDescription>
                  {ui("Nobody can reach this agent until Accept traffic is turned back on.")}
                </NoticeDescription>
              </Notice>
            )}
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

          <AgentGitHubSection
            agentId={agent.id}
            avatar={agent.avatar}
            githubTriggerCount={githubTriggerCount}
          />

          <AgentTriggersPanel agentId={agent.id}>
            {scheduleEndpoints.length > 0 && (
              <div className="flex flex-col gap-2">
                {scheduleEndpoints.map(({ channel }) => (
                  <ChannelRow
                    key={channel.id}
                    channel={channel}
                    expanded={expandedId === channel.id}
                    onToggle={() =>
                      setExpandedId((current) => (current === channel.id ? null : channel.id))
                    }
                    configureHref={
                      canManage && !sourceOwns(channel)
                        ? `/agents/${agent.id}/channels/${channel.id}`
                        : undefined
                    }
                    onPublishChange={
                      canDangerous
                        ? (publish) => publishEndpoint.mutate({ channelId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishEndpoint.isPending}
                    onRunNow={canManage ? () => triggerEndpoint.mutate(channel.id) : undefined}
                    usePanel={
                      canViewBudgets ? (
                        <BudgetPanel
                          subjectType="agent_channel"
                          subjectId={channel.id}
                          title={ui("Channel budget")}
                          canManage={canManageBudgets}
                        />
                      ) : undefined
                    }
                  />
                ))}
              </div>
            )}
          </AgentTriggersPanel>
        </PageMain>

        <PageRail>
          <RailSection label={ui("All channels")}>
            <div className="flex items-center justify-between gap-3">
              <label htmlFor="channels-enabled" className="text-sm font-medium">
                {ui("Accept traffic")}
              </label>
              <Switch
                id="channels-enabled"
                checked={!suspended}
                onCheckedChange={(enabled) =>
                  enabled ? resumeExposures.mutate(agent.id) : suspendExposures.mutate(agent.id)
                }
                disabled={!canManage || suspendExposures.isPending || resumeExposures.isPending}
              />
            </div>
            <p className="mt-3 text-sm text-muted-foreground">
              {ui(
                "Off pauses every channel at once. Each channel keeps its own Published or Draft setting.",
              )}
            </p>
          </RailSection>
          {canViewBudgets && (
            <RailSection label={ui("Agent budget")}>
              <BudgetPanel subjectType="agent" subjectId={agent.id} canManage={canManageBudgets} />
            </RailSection>
          )}
        </PageRail>
      </PageColumns>
    </div>
  );
}
