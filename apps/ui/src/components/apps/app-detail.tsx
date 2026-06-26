"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Archive, GlobeLock, Play, Plus, Rocket } from "lucide-react";
import { useAgents, usePageTitle } from "@/hooks";
import { useHarnesses } from "@/hooks/use-harnesses";
import { usePolicies } from "@/hooks/use-policies";
import { useApp, useDeleteApp, usePublishApp, useUnpublishApp } from "@/hooks/use-apps";
import { triggerChannel } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { Button, buttonVariants } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ChannelRow } from "@/components/apps/channel-row";
import { LiveActivityRail } from "@/components/apps/live-activity-rail";
import { MiniTimeline } from "@/components/apps/mini-timeline";
import { type StatStripStats } from "@/components/apps/stat-strip";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  StatCard,
  StatGrid,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import type { App, AppChannel } from "@/lib/api/types";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";

function buildStats(app: App): StatStripStats {
  const enabled = app.channels.filter((channel) => channel.enabled).length;
  const activeTriggers = app.channels.filter(
    (channel) =>
      channel.enabled && (channel.channel_type === "schedule" || channel.channel_type === "slack"),
  ).length;
  const endpoints = app.channels.filter(
    (channel) => channel.channel_type === "webhook" || channel.channel_type === "ag_ui",
  ).length;

  return {
    health: enabled === app.channels.length ? "Healthy" : "Needs attention",
    healthSub:
      app.channels.length === 0
        ? "No channels configured"
        : `${enabled} of ${app.channels.length} channels enabled`,
    invocations24h: 0,
    invocationSub: `${activeTriggers} active triggers · ${endpoints} endpoints`,
    successRate: null,
    successSub: "Run metrics pending backend aggregation",
    timeline: [],
  };
}

function findFirstRunnableChannel(app?: App): AppChannel | undefined {
  return app?.channels.find(
    (channel) =>
      channel.channel_type === "schedule" && channel.enabled && app.status === "published",
  );
}

export function AppDetail({ appId }: { appId: string }) {
  const queryClient = useQueryClient();
  const { data: app, isLoading } = useApp(appId);
  const { data: agents } = useAgents({ includeArchived: true });
  const { data: harnesses } = useHarnesses({ includeArchived: true });
  const { can } = usePolicies("apps");
  const deleteAppMutation = useDeleteApp();
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();
  const [expandedChannelId, setExpandedChannelId] = useState<string | null>(null);

  usePageTitle(app ? getDisplayName(app) : null, "App");

  const triggerMutation = useMutation({
    mutationFn: (channelId: string) => triggerChannel(appId, channelId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
    },
  });

  const agent = agents?.find((candidate) => candidate.id === app?.agent_id);
  const harness = harnesses?.find((candidate) => candidate.id === app?.harness_id);
  const stats = useMemo(() => (app ? buildStats(app) : null), [app]);
  const isReadOnly = isReadOnlyStatus(app?.status);
  const canManage = can("app.manage") && !isReadOnly;
  const canDangerous = can("app.dangerous") && !isReadOnly;
  const runnableChannel = findFirstRunnableChannel(app);

  if (isLoading) {
    return (
      <div className="container mx-auto space-y-4 p-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-28 w-full" />
        <Skeleton className="h-80 w-full" />
      </div>
    );
  }

  if (!app) {
    return (
      <ResourceNotFound
        title="App not found"
        description="This app may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/apps"
        backLabel="Back to apps"
        resourceId={appId}
      />
    );
  }

  const enabledChannels = app.channels.filter((channel) => channel.enabled).length;

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Apps", href: "/apps" }, { label: app.name }]} />

      <PageMasthead
        icon={<Rocket />}
        title={<span className={getEntityNameClassName(app.status)}>{app.name}</span>}
        badges={
          <>
            <CopyButton value={app.id} />
            <Badge variant={getEntityStatusBadgeVariant(app.status)}>{app.status}</Badge>
          </>
        }
        description={app.description || undefined}
        meta={
          <>
            <span>
              Agent{" "}
              {app.agent_id ? (
                <Link href={`/agents/${app.agent_id}`} className="text-foreground hover:underline">
                  {agent?.name ?? app.agent_id}
                </Link>
              ) : (
                <span className="text-foreground">unassigned</span>
              )}
            </span>
            <span>
              Harness{" "}
              <Link
                href={`/harnesses/${app.harness_id}`}
                className="text-foreground hover:underline"
              >
                {harness?.name ?? app.harness_id}
              </Link>
            </span>
          </>
        }
        actions={
          <>
            <Button
              variant="outline"
              onClick={() => runnableChannel && triggerMutation.mutate(runnableChannel.id)}
              disabled={!runnableChannel || triggerMutation.isPending || !canManage}
            >
              <Play className="size-4" />
              Test run
            </Button>
            {app.status === "published" ? (
              <Button
                variant="outline"
                onClick={() => unpublishApp.mutate(app.id)}
                disabled={unpublishApp.isPending || !canDangerous}
              >
                <GlobeLock className="size-4" />
                Unpublish
              </Button>
            ) : (
              <Button
                variant="accent"
                onClick={() => publishApp.mutate(app.id)}
                disabled={publishApp.isPending || !canDangerous}
              >
                Publish
              </Button>
            )}
            <Button
              variant="outline"
              onClick={() => deleteAppMutation.mutate(app.id)}
              disabled={deleteAppMutation.isPending || !canDangerous}
            >
              <Archive className="size-4" />
              Archive
            </Button>
          </>
        }
      />

      {stats && (
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
      )}

      <PageColumns>
        <PageMain>
          <section className="flex flex-col gap-3">
            <div className="flex items-end justify-between gap-4">
              <div>
                <h2 className="text-lg font-semibold tracking-tight">Channels</h2>
                <p className="text-sm text-muted-foreground">
                  {app.channels.length} {pluralize(app.channels.length, "channel")} ·{" "}
                  {enabledChannels} enabled
                </p>
              </div>
              {canManage && (
                <Link
                  href={`/apps/${app.id}/channels/new`}
                  className={buttonVariants({ variant: "outline", size: "sm" })}
                >
                  <Plus className="size-4" />
                  Add channel
                </Link>
              )}
            </div>

            {app.channels.length === 0 ? (
              <Card>
                <CardContent className="flex min-h-48 flex-col items-center justify-center gap-3 text-center">
                  <p className="text-sm text-muted-foreground">Add a channel to expose this app.</p>
                  {canManage && (
                    <Link
                      href={`/apps/${app.id}/channels/new`}
                      className={buttonVariants({ size: "sm" })}
                    >
                      <Plus className="size-4" />
                      Add channel
                    </Link>
                  )}
                </CardContent>
              </Card>
            ) : (
              <div className="flex flex-col gap-2">
                {app.channels.map((channel) => (
                  <ChannelRow
                    key={channel.id}
                    channel={channel}
                    app={app}
                    expanded={expandedChannelId === channel.id}
                    onToggle={() =>
                      setExpandedChannelId((current) =>
                        current === channel.id ? null : channel.id,
                      )
                    }
                    onRunNow={
                      canManage && !triggerMutation.isPending
                        ? () => triggerMutation.mutate(channel.id)
                        : undefined
                    }
                    configureHref={`/apps/${app.id}/channels/${channel.id}`}
                  />
                ))}
              </div>
            )}
          </section>
        </PageMain>

        <PageRail>
          <LiveActivityRail appId={app.id} />
        </PageRail>
      </PageColumns>

      <PageFooter>
        <BackLink href="/apps">Back to Apps</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
