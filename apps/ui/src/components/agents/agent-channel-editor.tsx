"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Check, Pencil, Play, Radio, Trash2 } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import {
  useAgentChannels,
  useDeleteAgentChannel,
  usePublishAgentChannel,
  useTriggerAgentChannel,
  useUpdateAgentChannel,
} from "@/hooks/use-agent-channels";
import { usePolicies } from "@/hooks/use-policies";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
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
import { getChannelTypeDisplayName, getChannelLifecyclePresentation } from "@/lib/app-channels";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";

export function AgentChannelEditor({ agentId, channelId }: { agentId: string; channelId: string }) {
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { channels, isLoading: channelsLoading } = useAgentChannels(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const channel = channels.find((entry) => entry.channel.id === channelId)?.channel;
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);
  const canDangerous =
    !policiesLoading && can("agent.dangerous") && !isReadOnlyStatus(agent?.status);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (agentLoading || channelsLoading || policiesLoading) {
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
    />
  );
}

function AgentChannelForm({
  agent,
  channel,
  canManage,
  canDangerous,
  returnHref,
}: {
  agent: Agent;
  channel: AppChannel;
  canManage: boolean;
  canDangerous: boolean;
  returnHref: string;
}) {
  const router = useRouter();
  const agentId = agent.id;
  const channelId = channel.id;
  const updateChannel = useUpdateAgentChannel(agentId, channelId);
  const deleteChannel = useDeleteAgentChannel(agentId, channelId);
  const publishChannel = usePublishAgentChannel(agentId);
  const triggerChannel = useTriggerAgentChannel(agentId);
  const [formState, setFormState] = useState<ChannelFormState>(() =>
    getDefaultChannelFormState(channel.channel_type, channel),
  );
  const agentName = getDisplayName(agent);
  const lifecycle = getChannelLifecyclePresentation(channel);
  const schedule =
    channel.channel_type === "schedule" ? (channel.channel_config as ScheduleChannelConfig) : null;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
          { label: `${getChannelTypeDisplayName(channel.channel_type)} channel` },
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
              disabled={!canManage || !isChannelFormValid(formState) || updateChannel.isPending}
            >
              <Check className="size-4" />
              {updateChannel.isPending ? "Saving..." : "Save"}
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => publishChannel.mutate({ channelId, publish: !lifecycle.isLive })}
              disabled={!canDangerous || !formState.enabled || publishChannel.isPending}
            >
              {lifecycle.isLive ? "Unpublish" : "Publish"}
            </Button>
            {channel.channel_type === "schedule" && (
              <Button
                type="button"
                variant="outline"
                onClick={() => triggerChannel.mutate(channelId)}
                disabled={!canManage || !lifecycle.isLive || triggerChannel.isPending}
              >
                <Play className="size-4" />
                Run now
              </Button>
            )}
            <Button
              type="button"
              variant="outline"
              onClick={() =>
                deleteChannel.mutate(undefined, {
                  onSuccess: () => router.push(returnHref),
                })
              }
              disabled={!canDangerous || deleteChannel.isPending}
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
          updateChannel.mutate(
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
            <Card>
              <CardContent className="py-5">
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="edit"
                  channelId={channel.id}
                />
              </CardContent>
            </Card>
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
    </PageContainer>
  );
}
