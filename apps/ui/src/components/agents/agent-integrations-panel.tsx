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
import { ChannelRow } from "@/components/agents/channels/channel-row";
import { MiniTimeline } from "@/components/apps/mini-timeline";
import { type StatStripStats } from "@/components/apps/stat-strip";
import { ChannelDetailsPanel } from "@/components/agents/integrations/channel-details-panel";
import { AgentGitHubCard } from "@/components/agents/agent-github-card";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
import {
  AgentVersionSelectionBadge,
  versionSelectionOf,
} from "@/components/agents/agent-version-policy-field";
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
import type { AgentChannelRow } from "@/hooks/use-agent-channels";
import { isReadOnlyStatus } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

function buildStats(
  channels: AgentChannelRow[],
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
  const publishEndpoint = usePublishAgentChannel(agent.id);
  const triggerEndpoint = useTriggerAgentChannel(agent.id);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const budgetsEnabled = useFeatureFlag("channel_budgets");

  const suspended = agent.exposures_suspended ?? false;
  const canManage = canAgent("agent.manage") && !isReadOnlyStatus(agent.status);
  const canDangerous = canAgent("agent.dangerous") && !isReadOnlyStatus(agent.status);
  const canViewBudgets = canBudget("budget.view");
  const canManageBudgets = canBudget("budget.manage");

  const stats = useMemo(
    () => buildStats(channels, triggers.length, suspended),
    [channels, suspended, triggers.length],
  );

  const doors = channels.filter(({ channel }) => !isTriggerChannel(channel));
  const scheduleEndpoints = channels.filter(({ channel }) => isTriggerChannel(channel));

  return (
    <div className="mx-auto flex w-full max-w-[1600px] flex-col gap-6">
      <div>
        <h2 className="text-xl font-semibold tracking-tight">Integrations</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Manage how this agent connects and runs.
        </p>
      </div>
      <PageControlStrip>
        <StatGrid className="gap-4">
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
          <StatCard label="Activity" hint="Run metrics pending backend aggregation">
            {stats.timeline.length > 0 ? (
              <MiniTimeline runs={stats.timeline} />
            ) : (
              <div className="flex h-8 items-center" aria-label="Activity unavailable">
                <span className="h-px w-full bg-border" />
              </div>
            )}
          </StatCard>
        </StatGrid>
      </PageControlStrip>

      <PageColumns>
        <PageMain className="gap-6">
          <section className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h3 className="text-lg font-semibold tracking-tight">Channels</h3>
                <p className="text-sm text-muted-foreground">How callers reach this agent</p>
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
                        <AgentVersionSelectionBadge
                          agentId={agent.id}
                          selection={versionSelectionOf(channel)}
                        />
                        <ChannelDetailsPanel
                          agentId={agent.id}
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
                        ? (publish) => publishEndpoint.mutate({ channelId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishEndpoint.isPending}
                  />
                ))}
              </div>
            )}
          </section>

          <AgentTriggersPanel agentId={agent.id}>
            <AgentGitHubCard agentId={agent.id} />
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
                      canManage ? `/agents/${agent.id}/channels/${channel.id}` : undefined
                    }
                    onPublishChange={
                      canDangerous
                        ? (publish) => publishEndpoint.mutate({ channelId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishEndpoint.isPending}
                    onRunNow={canManage ? () => triggerEndpoint.mutate(channel.id) : undefined}
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
          </AgentTriggersPanel>
        </PageMain>

        <PageRail>
          <RailSection label="Exposure">
            <div className="flex items-center justify-between gap-3">
              <label htmlFor="exposures-suspended" className="text-sm font-medium">
                Suspend all channels
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
            <p className="mt-3 text-sm text-muted-foreground">
              Temporarily pause all channels. Published settings are preserved.
            </p>
          </RailSection>
          {budgetsEnabled && canViewBudgets && (
            <RailSection label="Agent budget">
              <BudgetPanel subjectType="agent" subjectId={agent.id} canManage={canManageBudgets} />
            </RailSection>
          )}
        </PageRail>
      </PageColumns>
    </div>
  );
}
