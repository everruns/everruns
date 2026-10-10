"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Check, CircleAlert, Pencil, Play, Radio, Trash2 } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import {
  useAgentChannels,
  useDeleteAgentChannel,
  usePublishAgentChannel,
  useSlackInstallCapability,
  useTriggerAgentChannel,
  useUpdateAgentChannel,
} from "@/hooks/use-agent-channels";
import { usePolicies } from "@/hooks/use-policies";
import { Badge } from "@/components/ui/badge";
import { ChannelHealthWarning } from "@/components/health/channel-health-warning";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SlackRemovalNotice } from "@/components/agents/integrations/slack-removal-notice";
import { Card, CardContent } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import {
  buildChannelConfig,
  ChannelForm,
  getDefaultChannelFormState,
  isChannelFormValid,
  type ChannelFormState,
} from "@/components/agents/channels/channel-form";
import { AgentKeysCard } from "@/components/agents/channels/agent-keys-card";
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
import type { Agent, AgentChannel, ScheduleChannelConfig } from "@/lib/api/types";
import type { SlackInstallCapability } from "@/lib/api/agent-channels";
import { getChannelTypeDisplayName, getChannelLifecyclePresentation } from "@/lib/channel-display";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";

export function AgentChannelEditor({
  agentId,
  channelId,
  slackInstallFailure,
}: {
  agentId: string;
  channelId: string;
  slackInstallFailure?: string;
}) {
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { channels, isLoading: endpointsLoading } = useAgentChannels(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const slackInstallCapability = useSlackInstallCapability();
  const channel = channels.find(({ channel }) => channel.id === channelId)?.channel;
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);
  const canDangerous =
    !policiesLoading && can("agent.dangerous") && !isReadOnlyStatus(agent?.status);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (agentLoading || endpointsLoading || policiesLoading || slackInstallCapability.isLoading) {
    return <div className="container mx-auto p-6">Loading channel...</div>;
  }
  if (!agent || !channel) {
    return (
      <ResourceNotFound
        title="Channel not found"
        description="This channel may have been deleted, moved to another agent, or the URL may be wrong."
        backHref={returnHref}
        backLabel="Back to agent"
        resourceId={channelId}
      />
    );
  }

  return (
    <AgentChannelForm
      key={channel.id}
      agent={agent}
      channel={channel}
      canManage={canManage}
      canDangerous={canDangerous}
      returnHref={returnHref}
      slackInstallCapability={slackInstallCapability.data}
      onSlackCapabilityChanged={slackInstallCapability.refetch}
      slackInstallFailure={slackInstallFailure}
    />
  );
}

function AgentChannelForm({
  agent,
  channel,
  canManage,
  canDangerous,
  returnHref,
  slackInstallCapability,
  onSlackCapabilityChanged,
  slackInstallFailure,
}: {
  agent: Agent;
  channel: AgentChannel;
  canManage: boolean;
  canDangerous: boolean;
  returnHref: string;
  slackInstallCapability?: SlackInstallCapability;
  onSlackCapabilityChanged?: () => void | Promise<unknown>;
  slackInstallFailure?: string;
}) {
  const router = useRouter();
  const agentId = agent.id;
  const channelId = channel.id;
  const updateEndpoint = useUpdateAgentChannel(agentId, channelId);
  const deleteEndpoint = useDeleteAgentChannel(agentId, channelId);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const publishEndpoint = usePublishAgentChannel(agentId);
  const triggerEndpoint = useTriggerAgentChannel(agentId);
  const [formState, setFormState] = useState<ChannelFormState>(() =>
    getDefaultChannelFormState(channel.channel_type, channel),
  );
  const agentName = getDisplayName(agent);
  const lifecycle = getChannelLifecyclePresentation(channel);
  const slackInstallAvailable = slackInstallCapability?.connected === true;
  const slackInstallFailureMessage = slackInstallFailure
    ? /[.!?]$/.test(slackInstallFailure)
      ? slackInstallFailure
      : `${slackInstallFailure}.`
    : "Could not start the Slack install.";
  const schedule =
    channel.channel_type === "schedule" ? (channel.channel_config as ScheduleChannelConfig) : null;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
          {
            label: `${getChannelTypeDisplayName(channel.channel_type)} channel`,
          },
        ]}
      />
      <PageMasthead
        icon={<Radio />}
        title={`${getChannelTypeDisplayName(channel.channel_type)} channel`}
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
              form="channel-edit-form"
              disabled={!canManage || !isChannelFormValid(formState) || updateEndpoint.isPending}
            >
              <Check className="size-4" />
              {updateEndpoint.isPending ? "Saving..." : "Save"}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() =>
                publishEndpoint.mutate({
                  channelId,
                  publish: !lifecycle.isLive,
                })
              }
              disabled={!canDangerous || !formState.enabled || publishEndpoint.isPending}
            >
              {lifecycle.isLive ? "Unpublish" : "Publish"}
            </Button>
            {channel.channel_type === "schedule" && (
              <Button
                type="button"
                variant="outline"
                onClick={() => triggerEndpoint.mutate(channelId)}
                disabled={!canManage || !lifecycle.isLive || triggerEndpoint.isPending}
              >
                <Play className="size-4" />
                Run now
              </Button>
            )}
            <Button
              type="button"
              variant="outline"
              onClick={() => setConfirmDelete(true)}
              disabled={!canDangerous || deleteEndpoint.isPending}
            >
              <Trash2 className="size-4" />
              Delete
            </Button>
          </>
        }
      />
      <form
        id="channel-edit-form"
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
            {channel.channel_type === "slack" && <ChannelHealthWarning channelId={channel.id} />}
            {slackInstallFailure !== undefined && (
              <Notice variant="destructive" icon={<CircleAlert className="size-4" />} role="alert">
                <NoticeTitle>Slack install did not start</NoticeTitle>
                <NoticeDescription>
                  The channel was saved. {slackInstallFailureMessage}{" "}
                  {slackInstallAvailable
                    ? "Use Add to Slack to try again, or configure Slack manually."
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
                  channelId={channel.id}
                  channel={channel}
                  slackInstallCapability={slackInstallCapability}
                  onSlackCapabilityChanged={onSlackCapabilityChanged}
                />
              </CardContent>
            </Card>
            {channel.channel_type === "api" && (
              <AgentKeysCard agentId={agentId} channelId={channelId} canManage={canManage} />
            )}
          </PageMain>
          <PageRail>
            <RailSection label="Lifecycle">
              <p className="text-sm">{lifecycle.description}</p>
              <p className="mt-2 text-xs text-muted-foreground">
                Save configuration changes before publishing this channel.
              </p>
            </RailSection>
          </PageRail>
        </PageColumns>
      </form>
      <PageFooter>
        <BackLink href={returnHref}>Back to {agentName}</BackLink>
      </PageFooter>
      <Dialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              Delete {getChannelTypeDisplayName(channel.channel_type)} channel
            </DialogTitle>
            <DialogDescription>Delete this connection to {agentName}?</DialogDescription>
          </DialogHeader>
          <SlackRemovalNotice channels={[channel]} action="channel" />
          {deleteEndpoint.error && (
            <Notice variant="destructive">
              <NoticeDescription>{deleteEndpoint.error.message}</NoticeDescription>
            </Notice>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              disabled={deleteEndpoint.isPending}
              onClick={() => setConfirmDelete(false)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={!canDangerous || deleteEndpoint.isPending}
              onClick={() =>
                deleteEndpoint.mutate(undefined, { onSuccess: () => router.push(returnHref) })
              }
            >
              {deleteEndpoint.isPending ? "Deleting..." : "Delete channel"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}
