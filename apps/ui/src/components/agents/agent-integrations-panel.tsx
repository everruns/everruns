"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { usePublishChannel, useUnpublishChannel, useUpdateApp } from "@/hooks/use-apps";
import { useResumeAgentExposures, useSuspendAgentExposures } from "@/hooks/use-agents";
import { useAgentEndpoints, isTriggerChannel } from "@/hooks/use-agent-endpoints";
import { useAgentTriggers } from "@/hooks/use-agent-triggers";
import { usePolicies } from "@/hooks/use-policies";
import { triggerChannel } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { buttonVariants } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { ChannelRow } from "@/components/apps/channel-row";
import { LiveActivityRail } from "@/components/apps/live-activity-rail";
import { MiniTimeline } from "@/components/apps/mini-timeline";
import { type StatStripStats } from "@/components/apps/stat-strip";
import { EndpointUsePanel } from "@/components/agents/endpoint-use-panel";
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
import { AgentIdentitySelect } from "@/components/agent-identity/agent-identity-select";
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

/// The agent's exposures, in one place: how traffic reaches it (endpoints) and
/// when it wakes up on its own (triggers).
///
/// This absorbs the App detail page (EVE-1009). Endpoints belong to the agent
/// since EVE-1003, but the management API is still App-scoped, so each row
/// carries the App that owns its row and addresses writes through it. That
/// indirection disappears with the App domain (EVE-1011).
export function AgentIntegrationsPanel({ agent }: { agent: Agent }) {
  const queryClient = useQueryClient();
  const { endpoints, isLoading } = useAgentEndpoints(agent.id);
  const { data: triggers = [] } = useAgentTriggers(agent.id);
  const { can } = usePolicies("apps");
  const publishChannel = usePublishChannel();
  const unpublishChannel = useUnpublishChannel();
  const updateApp = useUpdateApp();
  const suspendExposures = useSuspendAgentExposures();
  const resumeExposures = useResumeAgentExposures();
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const suspended = agent.exposures_suspended ?? false;
  const canManage = can("app.manage");

  const triggerMutation = useMutation({
    mutationFn: ({ appId, channelId }: { appId: string; channelId: string }) =>
      triggerChannel(appId, channelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
    },
  });

  const stats = useMemo(
    () => buildStats(endpoints, triggers.length, suspended),
    [endpoints, suspended, triggers.length],
  );

  // An endpoint whose App row still carries the agent identity. Every App of
  // this agent should agree, so the first one is the one the control edits.
  const identityApp = endpoints[0]?.app;

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
                  className={buttonVariants({ variant: "outline", size: "sm" })}
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
                    This agent is not reachable from outside. Add an endpoint to expose it.
                  </p>
                  {canManage && (
                    <Link
                      href={`/agents/${agent.id}/endpoints/new`}
                      className={buttonVariants({ size: "sm" })}
                    >
                      <Plus className="size-4" />
                      Add endpoint
                    </Link>
                  )}
                </CardContent>
              </Card>
            ) : (
              <div className="flex flex-col gap-2">
                {doors.map(({ channel, app }) => (
                  <ChannelRow
                    key={channel.id}
                    channel={channel}
                    app={app}
                    expanded={expandedId === channel.id}
                    onToggle={() =>
                      setExpandedId((current) => (current === channel.id ? null : channel.id))
                    }
                    onPublishChange={
                      canManage
                        ? (publish) =>
                            (publish ? publishChannel : unpublishChannel).mutate({
                              appId: app.id,
                              channelId: channel.id,
                            })
                        : undefined
                    }
                    publishPending={publishChannel.isPending || unpublishChannel.isPending}
                    usePanel={<EndpointUsePanel channel={channel} />}
                    configureHref={`/agents/${agent.id}/endpoints/${channel.id}`}
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
                {scheduleEndpoints.map(({ channel, app }) => (
                  <ChannelRow
                    key={channel.id}
                    channel={channel}
                    app={app}
                    expanded={expandedId === channel.id}
                    onToggle={() =>
                      setExpandedId((current) => (current === channel.id ? null : channel.id))
                    }
                    onRunNow={
                      canManage && !triggerMutation.isPending
                        ? () => triggerMutation.mutate({ appId: app.id, channelId: channel.id })
                        : undefined
                    }
                    onPublishChange={
                      canManage
                        ? (publish) =>
                            (publish ? publishChannel : unpublishChannel).mutate({
                              appId: app.id,
                              channelId: channel.id,
                            })
                        : undefined
                    }
                    publishPending={publishChannel.isPending || unpublishChannel.isPending}
                    configureHref={`/agents/${agent.id}/endpoints/${channel.id}`}
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

          {identityApp && (
            <RailSection label="Agent identity">
              <AgentIdentitySelect
                value={identityApp.agent_identity_id ?? ""}
                onValueChange={(identityId) =>
                  updateApp.mutate({
                    appId: identityApp.id,
                    data: { agent_identity_id: identityId || null },
                  })
                }
                disabled={!canManage || updateApp.isPending}
                className="w-full"
              />
              <p className="mt-2 text-xs text-muted-foreground">
                Identity this agent presents when an endpoint starts a session.
              </p>
            </RailSection>
          )}

          {identityApp && <LiveActivityRail appId={identityApp.id} />}
        </PageRail>
      </PageColumns>
    </>
  );
}
