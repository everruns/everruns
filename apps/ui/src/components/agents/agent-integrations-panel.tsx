"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { Plus } from "lucide-react";
import { useResumeAgentExposures, useSuspendAgentExposures } from "@/hooks/use-agents";
import {
  isTriggerChannel,
  useAgentEndpoints,
  usePublishAgentEndpoint,
  useTriggerAgentEndpoint,
} from "@/hooks/use-agent-endpoints";
import { useAgentTriggers } from "@/hooks/use-agent-triggers";
import { usePolicies } from "@/hooks/use-policies";
import { Card, CardContent } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { buttonVariants } from "@/components/ui/button";
import { ChannelRow } from "@/components/apps/channel-row";
import { MiniTimeline } from "@/components/apps/mini-timeline";
import { type StatStripStats } from "@/components/apps/stat-strip";
import { EndpointDetailsPanel } from "@/components/agents/integrations/endpoint-details-panel";
import { AgentTriggersPanel } from "@/components/agents/agent-triggers-panel";
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
import type { AgentEndpoint } from "@/hooks/use-agent-endpoints";
import { pluralize } from "@/lib/formatting";

function buildStats(
  endpoints: AgentEndpoint[],
  triggerCount: number,
  suspended: boolean,
): StatStripStats {
  const live = endpoints.filter(
    ({ channel }) => channel.enabled && channel.status === "live",
  ).length;
  // Triggers come from two places while the migration is in flight: native
  // agent triggers, and the schedule channels that predate them. Counting only
  // the latter reported "0 triggers" for an agent that plainly had one.
  const scheduleChannels = endpoints.filter(({ channel }) => isTriggerChannel(channel)).length;
  const triggers = triggerCount + scheduleChannels;
  const doors = endpoints.length - scheduleChannels;

  let health: string;
  let healthSub: string;
  if (suspended) {
    health = "Suspended";
    healthSub = "Exposures are switched off for this agent";
  } else if (endpoints.length === 0) {
    health = "Not exposed";
    healthSub = "No endpoints configured";
  } else if (live === endpoints.length) {
    health = "Healthy";
    healthSub = `${live} of ${endpoints.length} endpoints live`;
  } else {
    health = "Needs attention";
    healthSub = `${live} of ${endpoints.length} endpoints live`;
  }

  return {
    health,
    healthSub,
    invocations24h: 0,
    invocationSub: `${triggers} ${pluralize(triggers, "trigger")} · ${doors} ${pluralize(doors, "endpoint")}`,
    successRate: null,
    successSub: "Run metrics pending backend aggregation",
    timeline: [],
  };
}

export function AgentIntegrationsPanel({ agent }: { agent: Agent }) {
  const { endpoints, isLoading } = useAgentEndpoints(agent.id);
  const { data: triggers = [] } = useAgentTriggers(agent.id);
  const { can } = usePolicies("agents");
  const suspendExposures = useSuspendAgentExposures();
  const resumeExposures = useResumeAgentExposures();
  const publishEndpoint = usePublishAgentEndpoint(agent.id);
  const triggerEndpoint = useTriggerAgentEndpoint(agent.id);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const suspended = agent.exposures_suspended ?? false;
  const canManage = can("agent.manage");
  const canDangerous = can("agent.dangerous");

  const stats = useMemo(
    () => buildStats(endpoints, triggers.length, suspended),
    [endpoints, suspended, triggers.length],
  );

  const doors = endpoints.filter(({ channel }) => !isTriggerChannel(channel));
  const scheduleEndpoints = endpoints.filter(({ channel }) => isTriggerChannel(channel));

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
                <h2 className="text-lg font-semibold tracking-tight">Endpoints</h2>
                <p className="text-sm text-muted-foreground">
                  {doors.length} {pluralize(doors.length, "endpoint")} · how callers reach this
                  agent
                </p>
              </div>
              {canManage && (
                <Link
                  href={`/agents/${agent.id}/endpoints/new`}
                  className={buttonVariants({ size: "sm" })}
                >
                  <Plus className="size-4" />
                  Add endpoint
                </Link>
              )}
            </div>

            {isLoading ? (
              <p className="text-sm text-muted-foreground">Loading endpoints…</p>
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
                      <EndpointDetailsPanel
                        agentName={agent.display_name ?? agent.name}
                        agentDescription={agent.description}
                        channel={channel}
                        configureHref={
                          canManage ? `/agents/${agent.id}/endpoints/${channel.id}` : undefined
                        }
                      />
                    }
                    configureHref={
                      canManage ? `/agents/${agent.id}/endpoints/${channel.id}` : undefined
                    }
                    onPublishChange={
                      canDangerous
                        ? (publish) => publishEndpoint.mutate({ endpointId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishEndpoint.isPending}
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
                      canManage ? `/agents/${agent.id}/endpoints/${channel.id}` : undefined
                    }
                    onPublishChange={
                      canDangerous
                        ? (publish) => publishEndpoint.mutate({ endpointId: channel.id, publish })
                        : undefined
                    }
                    publishPending={publishEndpoint.isPending}
                    onRunNow={canManage ? () => triggerEndpoint.mutate(channel.id) : undefined}
                  />
                ))}
              </div>
            )}

            <AgentTriggersPanel agentId={agent.id} />
          </section>
        </PageMain>

        <PageRail>
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
              Takes every endpoint of this agent off the internet at once, without changing the
              publish state each one should return to.
            </p>
          </RailSection>
        </PageRail>
      </PageColumns>
    </>
  );
}
