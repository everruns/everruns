"use client";

import { use, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Check, Radio } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import { useCreateAgentChannel, useSlackInstallCapability } from "@/hooks/use-agent-channels";
import { usePolicies } from "@/hooks/use-policies";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import {
  buildChannelConfig,
  ChannelForm,
  ChannelFormSummary,
  ChannelTypePicker,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/agents/channels/channel-form";
import {
  BackLink,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageFooter,
  PageMain,
  PageMasthead,
  PageRail,
} from "@/components/layout";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";
import { beginSlackInstall } from "@/lib/api/agent-channels";

export default function NewAgentChannelPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const router = useRouter();
  const { data: agent, isLoading } = useAgent(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const createChannel = useCreateAgentChannel(agentId);
  const slackInstallCapability = useSlackInstallCapability();
  const [formState, setFormState] = useState(() => getDefaultChannelFormState("webhook"));
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (isLoading || policiesLoading || slackInstallCapability.isLoading) {
    return <div className="container mx-auto p-6">Loading channel form...</div>;
  }
  if (!agent) {
    return (
      <ResourceNotFound
        title="Agent not found"
        description="This agent may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/agents"
        backLabel="Back to agents"
        resourceId={agentId}
      />
    );
  }

  const agentName = getDisplayName(agent);
  const slackInstallAvailable = slackInstallCapability.data?.connected === true;
  // Saving a credential-free Slack channel installs it straight away, so with
  // several workspaces connected the choice has to be made before Save, not
  // discovered as a failure after it. One workspace is selected automatically.
  const slackWorkspaceUnchosen =
    formState.kind === "slack" &&
    slackInstallAvailable &&
    !formState.slackCredentialsConfigured &&
    !formState.slackSigningSecret &&
    !formState.slackBotToken &&
    !formState.slackTeamId &&
    !formState.slackChannelId &&
    !formState.slackInstallTeamId;

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
          { label: "New channel" },
        ]}
      />
      <PageMasthead
        icon={<Radio />}
        title="New channel"
        description="Expose this agent through a webhook, AG-UI, Slack, or another transport."
        actions={
          <>
            <Button
              type="submit"
              form="channel-edit-form"
              disabled={
                !canManage ||
                !isChannelFormValid(formState) ||
                slackWorkspaceUnchosen ||
                createChannel.isPending
              }
            >
              <Check className="size-4" />
              {createChannel.isPending ? "Saving..." : "Save channel"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.push(returnHref)}>
              Discard
            </Button>
          </>
        }
      />
      <form
        id="channel-edit-form"
        onSubmit={(event) => {
          event.preventDefault();
          const hasManualSlackCredentials = Boolean(
            formState.slackCredentialsConfigured ||
            formState.slackSigningSecret ||
            formState.slackBotToken ||
            formState.slackTeamId ||
            formState.slackChannelId,
          );
          createChannel.mutate(
            {
              channel_type: formState.kind,
              channel_config: buildChannelConfig(formState),
              enabled: formState.enabled,
            },
            {
              onSuccess: async (channel) => {
                const endpointHref = `/agents/${agentId}/channels/${channel.id}`;
                if (
                  formState.kind !== "slack" ||
                  hasManualSlackCredentials ||
                  !slackInstallAvailable
                ) {
                  router.push(endpointHref);
                  return;
                }
                try {
                  const { authorize_url } = await beginSlackInstall(
                    channel.id,
                    formState.slackInstallTeamId || null,
                  );
                  window.location.href = authorize_url;
                } catch (caught) {
                  const reason =
                    caught instanceof Error ? caught.message : "Could not start the Slack install.";
                  router.push(
                    `${endpointHref}?slack_install=failed&reason=${encodeURIComponent(reason)}`,
                  );
                }
              },
            },
          );
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
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="new"
                  slackInstallCapability={slackInstallCapability.data}
                  onSlackCapabilityChanged={slackInstallCapability.refetch}
                />
              </CardContent>
            </Card>
          </PageMain>
          <PageRail>
            <ChannelFormSummary state={formState} />
          </PageRail>
        </PageColumns>
      </form>
      <PageFooter>
        <BackLink href={returnHref}>Back to {agentName}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
