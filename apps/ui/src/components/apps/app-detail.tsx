"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Archive, ArrowLeft, GlobeLock, Play, Plus, Rocket } from "lucide-react";
import { useAgents, usePageTitle } from "@/hooks";
import { useHarnesses } from "@/hooks/use-harnesses";
import { usePolicies } from "@/hooks/use-policies";
import { useApp, useDeleteApp, usePublishApp, useUnpublishApp } from "@/hooks/use-apps";
import { triggerChannel } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { Button, buttonVariants } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ResourceNotFound } from "@/components/resource-not-found";
import { ChannelRow } from "@/components/apps/channel-row";
import { LiveActivityRail } from "@/components/apps/live-activity-rail";
import { StatStrip, type StatStripStats } from "@/components/apps/stat-strip";
import type { App, AppChannel } from "@/lib/api/types";
import {
  getDisplayName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";

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

  return (
    <div className="container mx-auto space-y-6 p-6">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Link href="/apps" className="hover:text-foreground">
          Apps
        </Link>
        <span>/</span>
        <span>{app.name}</span>
      </div>

      <div className="flex flex-col gap-4 border-b pb-5 lg:flex-row lg:items-start lg:justify-between">
        <div className="flex min-w-0 gap-4">
          <span className="flex size-11 shrink-0 items-center justify-center border bg-accent/20">
            <Rocket className="size-5" />
          </span>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="text-2xl font-semibold">{app.name}</h1>
              <Badge variant={getEntityStatusBadgeVariant(app.status)}>{app.status}</Badge>
            </div>
            {app.description && <p className="mt-1 text-muted-foreground">{app.description}</p>}
            <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1 text-sm text-muted-foreground">
              <span>
                Agent{" "}
                {app.agent_id ? (
                  <Link
                    href={`/agents/${app.agent_id}`}
                    className="text-foreground hover:underline"
                  >
                    {agent?.name ?? app.agent_id}
                  </Link>
                ) : (
                  "unassigned"
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
            </div>
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
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
        </div>
      </div>

      {stats && <StatStrip stats={stats} />}

      <div className="grid gap-6 2xl:grid-cols-[minmax(0,1fr)_360px]">
        <section className="space-y-3">
          <div className="flex items-end justify-between gap-4">
            <div>
              <h2 className="text-lg font-semibold">Channels</h2>
              <p className="text-sm text-muted-foreground">
                {app.channels.length} channels ·{" "}
                {app.channels.filter((channel) => channel.enabled).length} enabled
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
            <div className="space-y-2">
              {app.channels.map((channel) => (
                <ChannelRow
                  key={channel.id}
                  channel={channel}
                  app={app}
                  expanded={expandedChannelId === channel.id}
                  onToggle={() =>
                    setExpandedChannelId((current) => (current === channel.id ? null : channel.id))
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

        <LiveActivityRail appId={app.id} />
      </div>

      <Link
        href="/apps"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="mr-2 size-4" />
        Back to Apps
      </Link>
    </div>
  );
}
