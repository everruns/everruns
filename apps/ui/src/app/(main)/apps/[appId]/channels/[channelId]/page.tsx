"use client";

import { use, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Play, Trash2 } from "lucide-react";
import { useApp } from "@/hooks/use-apps";
import { usePolicies } from "@/hooks/use-policies";
import { deleteChannel, triggerChannel, updateChannel } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ResourceNotFound } from "@/components/resource-not-found";
import { CronLabel } from "@/components/apps/cron-label";
import { MiniTimeline } from "@/components/apps/mini-timeline";
import {
  buildChannelConfig,
  ChannelForm,
  getDefaultChannelFormState,
  isChannelFormValid,
  type ChannelFormState,
} from "@/components/apps/channel-form";
import type { ScheduleChannelConfig } from "@/lib/api/types";
import { getChannelTypeDisplayName } from "@/lib/app-channels";
import { isReadOnlyStatus } from "@/lib/entity-lifecycle";

function channelTitle(state: ChannelFormState): string {
  if (state.kind === "schedule") return "Schedule channel";
  return `${getChannelTypeDisplayName(state.kind)} channel`;
}

export default function EditChannelPage({
  params,
}: {
  params: Promise<{ appId: string; channelId: string }>;
}) {
  const { appId, channelId } = use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const detailV2Enabled = useFeatureFlag("apps.detailV2");
  const { data: app, isLoading } = useApp(appId);
  const { can, isLoading: policiesLoading } = usePolicies("apps");
  const channel = app?.channels.find((candidate) => candidate.id === channelId);
  const [formState, setFormState] = useState<ChannelFormState | null>(null);
  const [activeTab, setActiveTab] = useState("schedule");
  const formStateKind = formState?.kind;
  const isReadOnly = isReadOnlyStatus(app?.status);
  const canManage = !policiesLoading && can("app.manage") && !isReadOnly;
  const canRunNow =
    canManage && formState?.kind === "schedule" && formState.enabled && app?.status === "published";

  useEffect(() => {
    if (!detailV2Enabled) router.replace(`/apps/${appId}`);
  }, [appId, detailV2Enabled, router]);

  useEffect(() => {
    if (app && !policiesLoading && !canManage) router.replace(`/apps/${appId}`);
  }, [app, appId, canManage, policiesLoading, router]);

  useEffect(() => {
    if (channel) setFormState(getDefaultChannelFormState(channel.channel_type, channel));
  }, [channel]);

  useEffect(() => {
    if (formStateKind && formStateKind !== "schedule" && activeTab === "schedule") {
      setActiveTab("invocation");
    }
  }, [activeTab, formStateKind]);

  const saveChannel = useMutation({
    mutationFn: () => {
      if (!canManage) throw new Error("Channel management is not available for this app");
      if (!formState) throw new Error("Missing channel form state");
      return updateChannel(appId, channelId, {
        channel_config: buildChannelConfig(formState),
        enabled: formState.enabled,
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
      router.push(`/apps/${appId}`);
    },
  });

  const toggleChannel = useMutation({
    mutationFn: (enabled: boolean) => {
      if (!canManage) throw new Error("Channel management is not available for this app");
      return updateChannel(appId, channelId, { enabled });
    },
    onMutate: (enabled) => {
      setFormState((current) => (current ? { ...current, enabled } : current));
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) }),
  });

  const removeChannel = useMutation({
    mutationFn: () => {
      if (!canManage) throw new Error("Channel management is not available for this app");
      return deleteChannel(appId, channelId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
      router.push(`/apps/${appId}`);
    },
  });

  const runNow = useMutation({
    mutationFn: () => {
      if (!canRunNow) throw new Error("Channel cannot be triggered right now");
      return triggerChannel(appId, channelId);
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) }),
  });

  const subline = useMemo(() => {
    if (!formState || !channel) return "";
    if (formState.kind === "schedule") {
      const config = channel.channel_config as ScheduleChannelConfig;
      return (
        <>
          Schedule · <CronLabel expr={config.cron_expression} tz={config.timezone} /> ·{" "}
          {!formState.enabled
            ? "Paused"
            : app?.status === "published"
              ? "Active"
              : "Active when published"}
        </>
      );
    }
    return `${getChannelTypeDisplayName(formState.kind)} · ${formState.enabled ? "Enabled" : "Disabled"}`;
  }, [app?.status, channel, formState]);

  if (!detailV2Enabled) {
    return null;
  }

  if (isLoading || policiesLoading || !formState)
    return <div className="container mx-auto p-6">Loading channel...</div>;
  if (!app || !channel) {
    return (
      <ResourceNotFound
        title="Channel not found"
        description="This channel may have been deleted, moved to another app, or the URL may be wrong."
        backHref={`/apps/${appId}`}
        backLabel="Back to app"
        resourceId={channelId}
      />
    );
  }

  return (
    <div className="min-h-[calc(100vh-3rem)]">
      <div className="border-b bg-background">
        <div className="container mx-auto px-6 py-4">
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Link href="/apps" className="hover:text-foreground">
              Apps
            </Link>
            <span>/</span>
            <Link href={`/apps/${app.id}`} className="hover:text-foreground">
              {app.name}
            </Link>
            <span>/</span>
            <span>{channelTitle(formState)}</span>
          </div>
          <div className="mt-4 flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
            <div className="flex items-start gap-4">
              <Button
                type="button"
                variant="ghost"
                size="icon"
                onClick={() => router.push(`/apps/${app.id}`)}
              >
                <ArrowLeft className="size-4" />
              </Button>
              <div>
                <div className="flex items-center gap-2">
                  <h1 className="text-2xl font-semibold">{channelTitle(formState)}</h1>
                  <Badge variant={formState.enabled ? "default" : "secondary"}>
                    {formState.enabled ? "active" : "paused"}
                  </Badge>
                </div>
                <p className="mt-1 text-muted-foreground">{subline}</p>
              </div>
            </div>
            <div className="flex gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => toggleChannel.mutate(!formState.enabled)}
                disabled={!canManage || toggleChannel.isPending}
              >
                {formState.enabled ? "Pause" : "Enable"}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => runNow.mutate()}
                disabled={!canRunNow || runNow.isPending}
              >
                <Play className="size-4" />
                Run now
              </Button>
            </div>
          </div>
        </div>
      </div>

      <div className="container mx-auto grid gap-6 px-6 py-6 lg:grid-cols-[minmax(0,1fr)_320px]">
        <Card>
          <CardContent className="py-4">
            <Tabs value={activeTab} onValueChange={setActiveTab}>
              <TabsList>
                {formState.kind === "schedule" && (
                  <TabsTrigger value="schedule">Schedule</TabsTrigger>
                )}
                <TabsTrigger value="invocation">Invocation</TabsTrigger>
                <TabsTrigger value="session">Session</TabsTrigger>
                <TabsTrigger value="runs">Runs</TabsTrigger>
              </TabsList>
              {formState.kind === "schedule" && (
                <TabsContent value="schedule" className="mt-6">
                  <ChannelForm
                    state={formState}
                    onChange={setFormState}
                    mode="edit"
                    section="schedule"
                  />
                </TabsContent>
              )}
              <TabsContent value="invocation" className="mt-6">
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="edit"
                  section="invocation"
                />
              </TabsContent>
              <TabsContent value="session" className="mt-6">
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="edit"
                  section="session"
                />
              </TabsContent>
              <TabsContent value="runs" className="mt-6">
                <ChannelForm state={formState} onChange={setFormState} mode="edit" section="runs" />
              </TabsContent>
            </Tabs>
          </CardContent>
        </Card>

        <Card className="h-fit">
          <CardHeader>
            <CardTitle>24h stats</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-2 gap-3 text-sm">
              <div className="border p-3">
                <p className="text-xs uppercase text-muted-foreground">Runs</p>
                <p className="mt-1 text-xl font-semibold">0</p>
              </div>
              <div className="border p-3">
                <p className="text-xs uppercase text-muted-foreground">Errors</p>
                <p className="mt-1 text-xl font-semibold">0</p>
              </div>
              <div className="border p-3">
                <p className="text-xs uppercase text-muted-foreground">p95</p>
                <p className="mt-1 text-xl font-semibold">--</p>
              </div>
              <div className="border p-3">
                <p className="text-xs uppercase text-muted-foreground">Success</p>
                <p className="mt-1 text-xl font-semibold">--</p>
              </div>
            </div>
            <MiniTimeline />
            <div className="text-sm text-muted-foreground">
              <p>Created {new Date(channel.created_at).toLocaleString()}</p>
              <p>Updated {new Date(channel.updated_at).toLocaleString()}</p>
            </div>
          </CardContent>
        </Card>
      </div>

      <div className="sticky bottom-0 border-t bg-background/95 px-6 py-3 backdrop-blur">
        <div className="container mx-auto flex items-center justify-between gap-2">
          <Button
            type="button"
            variant="destructive"
            onClick={() => removeChannel.mutate()}
            disabled={!canManage || removeChannel.isPending}
          >
            <Trash2 className="size-4" />
            Delete
          </Button>
          <div className="flex gap-2">
            <Button type="button" variant="outline" onClick={() => router.push(`/apps/${app.id}`)}>
              Cancel
            </Button>
            <Button
              type="button"
              disabled={!canManage || !isChannelFormValid(formState) || saveChannel.isPending}
              onClick={() => saveChannel.mutate()}
            >
              {saveChannel.isPending ? "Saving..." : "Save"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
