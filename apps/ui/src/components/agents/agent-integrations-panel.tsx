"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { Plus } from "lucide-react";
import { useResumeAgentExposures, useSuspendAgentExposures } from "@/hooks/use-agents";
import {
  isTriggerChannel,
  useAgentChannels,
  usePublishAgentChannel,
  useTriggerAgentChannel,
} from "@/hooks/use-agent-channels";
import { useAgentTriggers } from "@/hooks/use-agent-triggers";
import { usePolicies } from "@/hooks/use-policies";
import { Card, CardContent } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { buttonVariants } from "@/components/ui/button";
import { ChannelRow } from "@/components/apps/channel-row";
import { MiniTimeline } from "@/components/apps/mini-timeline";
import { type StatStripStats } from "@/components/apps/stat-strip";
import { ChannelDetailsPanel } from "@/components/agents/integrations/channel-details-panel";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
import { BudgetPanel } from "@/components/budgets/budget-panel";
import {
  PageControlStrip,
  StatCard,
  StatGrid,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
} from "@/components/layout";
import type { Agent } from "@/lib/api/types";
import type { AgentChannel } from "@/hooks/use-agent-channels";
import { pluralize } from "@/lib/formatting";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

function buildStats(
  channels: AgentChannel[],
  triggerCount: number,
  suspended: boolean,
): StatStripStats {
  const live = channels.filter(
    ({ channel }) => channel.enabled && channel.status === "live",
  ).length;
  // Triggers come from two places while the migration is in flight: native
  // agent triggers, and the schedule channels that predate them. Counting only
  // the latter reported "0 triggers" for an agent that plainly had one.
  const scheduleChannels = channels.filter(({ channel }) => isTriggerChannel(channel)).length;
  const triggers = triggerCount + scheduleChannels;
  const doors = channels.length - scheduleChannels;

  let health: string;
  let healthSub: string;
  if (suspended) {
    health = "Suspended";
    healthSub = "Exposures are switched off for this agent";
  } else if (channels.length === 0) {
    health = "Not exposed";
    healthSub = "No channels configured";
  } else if (live === channels.length) {
    health = "Healthy";
    healthSub = `${live} of ${channels.length} channels live`;
  } else {
    health = "Needs attention";
    healthSub = `${live} of ${channels.length} channels live`;
  }

  return {
    health,
    healthSub,
    invocations24h: 0,
    invocationSub: `${triggers} ${pluralize(triggers, "trigger")} · ${doors} ${pluralize(doors, "channel")}`,
    successRate: null,
    successSub: "Run metrics pending backend aggregation",
    timeline: [],
  };
}

export function AgentIntegrationsPanel({ agent }: { agent: Agent }) {
  const { channels, isLoading } = useAgentChannels(agent.id);
  const { data: triggers = [] } = useAgentTriggers(agent.id);
  const { can: canAgent } = usePolicies("agents");
  const { can: canBudget } = usePolicies("budgets");
  const suspendExposures = useSuspendAgentExposures();
  const resumeExposures = useResumeAgentExposures();
  const publishChannel = usePublishAgentChannel(agent.id);
  const triggerChannel = useTriggerAgentChannel(agent.id);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const budgetsEnabled = useFeatureFlag("app_budgets");

  const suspended = agent.exposures_suspended ?? false;
  const canManage = canAgent("agent.manage");
  const canDangerous = canAgent("agent.dangerous");
  const canViewBudgets = canBudget("budget.view");
  const canManageBudgets = canBudget("budget.manage");

  const stats = useMemo(
    () => buildStats(channels, triggers.length, suspended),
    [channels, suspended, triggers.length],
  );

  const doors = channels.filter(({ channel }) => !isTriggerChannel(channel));
  const scheduleTriggerChannels = channels.filter(({ channel }) => isTriggerChannel(channel));

  return (
    <>
      <PageControlStrip>
        <StatGrid>
          <StatCard label="Health" value={stats.health} hint={stats.healthSub} />
          <StatCard
            label="Invocations · 24h"
            value={String(stats.invocations24h)}
            hint={stats.invocationSub}
          />
          <StatCard
            label="Success rate"
            value={stats.successRate === null ? "No runs" : `${stats.successRate.toFixed(1)}%`}
            hint={stats.successSub}
          />
          <StatCard label="Activity" hint="Last 24 hours">
            <MiniTimeline runs={stats.timeline} />
          </StatCard>
        </StatGrid>
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          <section className="flex flex-col gap-3">
            <div className="flex items-end justify-between gap-4">
              <div>
                <h2 className="text-lg font-semibold tracking-tight">Channels</h2>
                <p className="text-sm text-muted-foreground">
                  {doors.length} {pluralize(doors.length, "channel")} · how callers reach this agent
                </p>
              </div>
              {canManage && (
                <Link
                  href={`/agents/${agent.id}/channels/new`}
                  className={buttonVariants({ size: "sm" })}
                >
                  <Plus className="size-4" />
                  Add channel
                </Link>
              )}
            </div>

            {isLoading ? (
              <p className="text-sm text-muted-foreground">Loading channels…</p>
            ) : doors.length === 0 ? (
              <Card>
                <CardContent className="flex min-h-48 flex-col items-center justify-center gap-3 text-center">
                  <p className="text-sm text-muted-foreground">
                    This agent is not reachable from outside.
                  </p>
                </CardContent>
              </Card>
            ) : (
              <div className="flex flex-col gap-2">
                {doors.map(({ channel }) => (
                  <ChannelRow
                    key={channel.id}
                    channel={channel}
                    expanded={expandedId === channel.id}
                    onToggle={() =>
                      setExpandedId((current) => (current === channel.id ? null : channel.id))
                    }
                    usePanel={
                      <div className="space-y-4">
                        <ChannelDetailsPanel
                          agentName={agent.display_name ?? agent.name}
                          agentDescription={agent.description}
                          channel={channel}
                          configureHref={
                            canManage ? `/agents/${agent.id}/channels/${channel.id}` : undefined
                          }
                        />
                        {budgetsEnabled && canViewBudgets && (
                          <div className="border-t pt-4">
                            <BudgetPanel
                              subjectType="agent_channel"
                              subjectId={channel.id}
                              title="Channel budget"
                              canManage={canManageBudgets}
                            />
                          </div>
                        )}
                      </div>
                    }
                    configureHref={
                      canManage ? `/agents/${agent.id}/channels/${channel.id}` : undefined
                    }
                    onPublishChange={
                      canDangerous
                        ? (publish) => publishChannel.mutate({ channelId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishChannel.isPending}
                  />
                ))}
              </div>
            )}
          </section>

          <section className="flex flex-col gap-3">
            <div>
              <h2 className="text-lg font-semibold tracking-tight">Triggers</h2>
              <p className="text-sm text-muted-foreground">When this agent wakes up on its own</p>
            </div>

            {scheduleTriggerChannels.length > 0 && (
              <div className="flex flex-col gap-2">
                {scheduleTriggerChannels.map(({ channel }) => (
                  <ChannelRow
                    key={channel.id}
                    channel={channel}
                    expanded={expandedId === channel.id}
                    onToggle={() =>
                      setExpandedId((current) => (current === channel.id ? null : channel.id))
                    }
                    configureHref={
                      canManage ? `/agents/${agent.id}/channels/${channel.id}` : undefined
                    }
                    onPublishChange={
                      canDangerous
                        ? (publish) => publishChannel.mutate({ channelId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishChannel.isPending}
                    onRunNow={canManage ? () => triggerChannel.mutate(channel.id) : undefined}
                    usePanel={
                      budgetsEnabled && canViewBudgets ? (
                        <BudgetPanel
                          subjectType="agent_channel"
                          subjectId={channel.id}
                          title="Channel budget"
                          canManage={canManageBudgets}
                        />
                      ) : undefined
                    }
                  />
                ))}
              </div>
            )}

            <AgentTriggersPanel agentId={agent.id} />
          </section>
        </PageMain>

        <PageRail>
          {budgetsEnabled && canViewBudgets && (
            <RailSection label="Agent budget">
              <BudgetPanel subjectType="agent" subjectId={agent.id} canManage={canManageBudgets} />
            </RailSection>
          )}
          <RailSection label="Exposures">
            <div className="flex items-center justify-between gap-3">
              <label htmlFor="exposures-suspended" className="text-sm">
                Suspend all
              </label>
              <Switch
                id="exposures-suspended"
                checked={suspended}
                onCheckedChange={(next) =>
                  next ? suspendExposures.mutate(agent.id) : resumeExposures.mutate(agent.id)
                }
                disabled={!canManage || suspendExposures.isPending || resumeExposures.isPending}
              />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              Takes every channel of this agent off the internet at once, without changing the
              publish state each one should return to.
            </p>
          </RailSection>
        </PageRail>
      </PageColumns>
    </>
  );
}
