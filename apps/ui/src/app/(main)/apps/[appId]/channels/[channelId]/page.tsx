"use client";

import { use, useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Pencil, Play, Radio, Trash2 } from "lucide-react";
import { useApp } from "@/hooks/use-apps";
import { usePolicies } from "@/hooks/use-policies";
import { deleteChannel, triggerChannel, updateChannel } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
  type SectionTabItem,
} from "@/components/layout";
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

  const tabItems: SectionTabItem[] = [
    ...(formState.kind === "schedule" ? [{ value: "schedule", label: "Schedule" }] : []),
    { value: "invocation", label: "Invocation" },
    { value: "session", label: "Session" },
    { value: "runs", label: "Runs" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Apps", href: "/apps" },
          { label: app.name, href: `/apps/${app.id}` },
          { label: channelTitle(formState) },
        ]}
      />

      <PageMasthead
        icon={<Radio />}
        title={channelTitle(formState)}
        badges={
          <>
            <Badge variant="accent">
              <Pencil className="size-3" />
              Editing
            </Badge>
            <Badge variant={formState.enabled ? "default" : "secondary"}>
              {formState.enabled ? "active" : "paused"}
            </Badge>
          </>
        }
        description={subline}
        actions={
          <>
            <Button
              type="submit"
              form="channel-edit-form"
              disabled={!canManage || !isChannelFormValid(formState) || saveChannel.isPending}
            >
              <Check className="size-4" />
              {saveChannel.isPending ? "Saving..." : "Save"}
            </Button>
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
            <Button type="button" variant="outline" onClick={() => router.push(`/apps/${app.id}`)}>
              Discard
            </Button>
          </>
        }
      />

      <PageControlStrip>
        <SectionTabs value={activeTab} onValueChange={setActiveTab} items={tabItems} />
      </PageControlStrip>

      <form
        id="channel-edit-form"
        onSubmit={(e) => {
          e.preventDefault();
          saveChannel.mutate();
        }}
      >
        <PageColumns>
          <PageMain>
            <Card>
              <CardContent className="py-4">
                {activeTab === "schedule" && formState.kind === "schedule" && (
                  <ChannelForm
                    state={formState}
                    onChange={setFormState}
                    mode="edit"
                    section="schedule"
                  />
                )}
                {activeTab === "invocation" && (
                  <ChannelForm
                    state={formState}
                    onChange={setFormState}
                    mode="edit"
                    section="invocation"
                  />
                )}
                {activeTab === "session" && (
                  <ChannelForm
                    state={formState}
                    onChange={setFormState}
                    mode="edit"
                    section="session"
                  />
                )}
                {activeTab === "runs" && (
                  <ChannelForm
                    state={formState}
                    onChange={setFormState}
                    mode="edit"
                    section="runs"
                  />
                )}
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
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

            <Card className="h-fit border-destructive/50">
              <CardHeader>
                <CardTitle className="text-destructive">Danger zone</CardTitle>
              </CardHeader>
              <CardContent>
                <Button
                  type="button"
                  variant="destructive"
                  onClick={() => removeChannel.mutate()}
                  disabled={!canManage || removeChannel.isPending}
                >
                  <Trash2 className="size-4" />
                  Delete channel
                </Button>
              </CardContent>
            </Card>
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href={`/apps/${app.id}`}>Back to {app.name}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
