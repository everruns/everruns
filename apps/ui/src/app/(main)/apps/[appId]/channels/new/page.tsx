"use client";

import { use, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Radio } from "lucide-react";
import { useApp } from "@/hooks/use-apps";
import { usePolicies } from "@/hooks/use-policies";
import { addChannel } from "@/lib/api/apps";
import { queryKeys } from "@/lib/query-keys";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import {
  buildChannelConfig,
  ChannelForm,
  ChannelFormSummary,
  ChannelTypePicker,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/apps/channel-form";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import { isReadOnlyStatus } from "@/lib/entity-lifecycle";

export default function NewChannelPage({ params }: { params: Promise<{ appId: string }> }) {
  const { appId } = use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data: app, isLoading } = useApp(appId);
  const { can, isLoading: policiesLoading } = usePolicies("apps");
  const [formState, setFormState] = useState(() => getDefaultChannelFormState("webhook"));
  const isReadOnly = isReadOnlyStatus(app?.status);
  const canManage = !policiesLoading && can("app.manage") && !isReadOnly;

  useEffect(() => {
    if (app && !policiesLoading && !canManage) router.replace(`/apps/${appId}`);
  }, [app, appId, canManage, policiesLoading, router]);

  const createChannel = useMutation({
    mutationFn: () => {
      if (!canManage) throw new Error("Channel management is not available for this app");
      return addChannel(appId, {
        channel_type: formState.kind,
        channel_config: buildChannelConfig(formState),
        enabled: formState.enabled,
      });
    },
    onSuccess: (channel) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
      router.push(`/apps/${appId}/channels/${channel.id}`);
    },
  });

  if (isLoading || policiesLoading)
    return <div className="container mx-auto p-6">Loading channel form...</div>;
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
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Apps", href: "/apps" },
          { label: app.name, href: `/apps/${app.id}` },
          { label: "New channel" },
        ]}
      />

      <PageMasthead
        icon={<Radio />}
        title="New channel"
        badges={<Badge variant="outline">{app.status}</Badge>}
        description="Invoke this app via webhook, through AG-UI, or from Slack. Configure schedules on the agent's Triggers tab."
        actions={
          <>
            <Button
              type="submit"
              form="channel-edit-form"
              disabled={!canManage || !isChannelFormValid(formState) || createChannel.isPending}
            >
              <Check className="size-4" />
              {createChannel.isPending ? "Saving..." : "Save channel"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.push(`/apps/${app.id}`)}>
              Discard
            </Button>
          </>
        }
      />

      <form
        id="channel-edit-form"
        onSubmit={(e) => {
          e.preventDefault();
          createChannel.mutate();
        }}
      >
        <PageColumns>
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>1. Channel type</CardTitle>
              </CardHeader>
              <CardContent>
                <ChannelTypePicker
                  value={formState.kind}
                  onChange={(kind) => setFormState(getDefaultChannelFormState(kind))}
                />
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>
                  2. Configure {formState.kind === "ag_ui" ? "AG-UI" : formState.kind}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ChannelForm state={formState} onChange={setFormState} mode="new" />
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
            <ChannelFormSummary app={app} state={formState} />
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href={`/apps/${app.id}`}>Back to {app.name}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
