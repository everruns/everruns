"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Check, CircleAlert, Pencil, Play, Radio, Trash2 } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import {
  useAgentEndpoints,
  useDeleteAgentEndpoint,
  usePublishAgentEndpoint,
  useSlackInstallCapability,
  useTriggerAgentEndpoint,
  useUpdateAgentEndpoint,
} from "@/hooks/use-agent-endpoints";
import { usePolicies } from "@/hooks/use-policies";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import {
  buildChannelConfig,
  ChannelForm,
  getDefaultChannelFormState,
  isChannelFormValid,
  type ChannelFormState,
} from "@/components/apps/channel-form";
import { CronLabel } from "@/components/apps/cron-label";
import {
  BackLink,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageFooter,
  PageMain,
  PageMasthead,
  PageRail,
  RailSection,
} from "@/components/layout";
import type { Agent, AppChannel, ScheduleChannelConfig } from "@/lib/api/types";
import type { SlackInstallCapability } from "@/lib/api/agent-endpoints";
import { getChannelTypeDisplayName, getEndpointLifecyclePresentation } from "@/lib/app-channels";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";

export function AgentEndpointEditor({
  agentId,
  endpointId,
  slackInstallFailure,
}: {
  agentId: string;
  endpointId: string;
  slackInstallFailure?: string;
}) {
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { endpoints, isLoading: endpointsLoading } = useAgentEndpoints(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const slackInstallCapability = useSlackInstallCapability();
  const endpoint = endpoints.find(({ channel }) => channel.id === endpointId)?.channel;
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);
  const canDangerous =
    !policiesLoading && can("agent.dangerous") && !isReadOnlyStatus(agent?.status);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (agentLoading || endpointsLoading || policiesLoading || slackInstallCapability.isLoading) {
    return <div className="container mx-auto p-6">Loading endpoint...</div>;
  }
  if (!agent || !endpoint) {
    return (
      <ResourceNotFound
        title="Endpoint not found"
        description="This endpoint may have been deleted, moved to another agent, or the URL may be wrong."
        backHref={returnHref}
        backLabel="Back to agent"
        resourceId={endpointId}
      />
    );
  }

  return (
    <AgentEndpointForm
      key={endpoint.id}
      agent={agent}
      endpoint={endpoint}
      canManage={canManage}
      canDangerous={canDangerous}
      returnHref={returnHref}
      slackInstallCapability={slackInstallCapability.data}
      onSlackCapabilityChanged={slackInstallCapability.refetch}
      slackInstallFailure={slackInstallFailure}
    />
  );
}

function AgentEndpointForm({
  agent,
  endpoint,
  canManage,
  canDangerous,
  returnHref,
  slackInstallCapability,
  onSlackCapabilityChanged,
  slackInstallFailure,
}: {
  agent: Agent;
  endpoint: AppChannel;
  canManage: boolean;
  canDangerous: boolean;
  returnHref: string;
  slackInstallCapability?: SlackInstallCapability;
  onSlackCapabilityChanged?: () => void | Promise<unknown>;
  slackInstallFailure?: string;
}) {
  const router = useRouter();
  const agentId = agent.id;
  const endpointId = endpoint.id;
  const updateEndpoint = useUpdateAgentEndpoint(agentId, endpointId);
  const deleteEndpoint = useDeleteAgentEndpoint(agentId, endpointId);
  const publishEndpoint = usePublishAgentEndpoint(agentId);
  const triggerEndpoint = useTriggerAgentEndpoint(agentId);
  const [formState, setFormState] = useState<ChannelFormState>(() =>
    getDefaultChannelFormState(endpoint.channel_type, endpoint),
  );
  const agentName = getDisplayName(agent);
  const lifecycle = getEndpointLifecyclePresentation(endpoint);
  const slackInstallAvailable = slackInstallCapability?.connected === true;
  const slackInstallFailureMessage = slackInstallFailure
    ? /[.!?]$/.test(slackInstallFailure)
      ? slackInstallFailure
      : `${slackInstallFailure}.`
    : "Could not start the Slack install.";
  const schedule =
    endpoint.channel_type === "schedule"
      ? (endpoint.channel_config as ScheduleChannelConfig)
      : null;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
          { label: `${getChannelTypeDisplayName(endpoint.channel_type)} endpoint` },
        ]}
      />
      <PageMasthead
        icon={<Radio />}
        title={`${getChannelTypeDisplayName(endpoint.channel_type)} endpoint`}
        badges={
          <>
            <Badge variant="accent">
              <Pencil className="size-3" />
              Editing
            </Badge>
            <Badge variant={lifecycle.isLive ? "default" : "secondary"}>{lifecycle.label}</Badge>
          </>
        }
        description={
          schedule ? (
            <>
              Schedule · <CronLabel expr={schedule.cron_expression} tz={schedule.timezone} /> ·{" "}
              {lifecycle.description}
            </>
          ) : (
            lifecycle.description
          )
        }
        actions={
          <>
            <Button
              type="submit"
              form="endpoint-edit-form"
              disabled={!canManage || !isChannelFormValid(formState) || updateEndpoint.isPending}
            >
              <Check className="size-4" />
              {updateEndpoint.isPending ? "Saving..." : "Save"}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => publishEndpoint.mutate({ endpointId, publish: !lifecycle.isLive })}
              disabled={!canDangerous || !formState.enabled || publishEndpoint.isPending}
            >
              {lifecycle.isLive ? "Unpublish" : "Publish"}
            </Button>
            {endpoint.channel_type === "schedule" && (
              <Button
                type="button"
                variant="outline"
                onClick={() => triggerEndpoint.mutate(endpointId)}
                disabled={!canManage || !lifecycle.isLive || triggerEndpoint.isPending}
              >
                <Play className="size-4" />
                Run now
              </Button>
            )}
            <Button
              type="button"
              variant="outline"
              onClick={() =>
                deleteEndpoint.mutate(undefined, {
                  onSuccess: () => router.push(returnHref),
                })
              }
              disabled={!canDangerous || deleteEndpoint.isPending}
            >
              <Trash2 className="size-4" />
              Delete
            </Button>
          </>
        }
      />
      <form
        id="endpoint-edit-form"
        onSubmit={(event) => {
          event.preventDefault();
          updateEndpoint.mutate(
            {
              channel_config: buildChannelConfig(formState),
              enabled: formState.enabled,
            },
            { onSuccess: () => router.push(returnHref) },
          );
        }}
      >
        <PageColumns>
          <PageMain>
            {slackInstallFailure !== undefined && (
              <Notice variant="destructive" icon={<CircleAlert className="size-4" />} role="alert">
                <NoticeTitle>Slack install did not start</NoticeTitle>
                <NoticeDescription>
                  The endpoint was saved. {slackInstallFailureMessage}{" "}
                  {slackInstallAvailable
                    ? "Use Connect to Slack to try again, or configure Slack manually."
                    : "Configure Slack manually."}
                </NoticeDescription>
              </Notice>
            )}
            <Card>
              <CardContent className="py-5">
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="edit"
                  endpointId={endpoint.id}
                  slackInstallCapability={slackInstallCapability}
                  onSlackCapabilityChanged={onSlackCapabilityChanged}
                />
              </CardContent>
            </Card>
          </PageMain>
          <PageRail>
            <RailSection label="Lifecycle">
              <p className="text-sm">{lifecycle.description}</p>
              <p className="mt-2 text-xs text-muted-foreground">
                Save configuration changes before publishing this endpoint.
              </p>
            </RailSection>
          </PageRail>
        </PageColumns>
      </form>
      <PageFooter>
        <BackLink href={returnHref}>Back to {agentName}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
