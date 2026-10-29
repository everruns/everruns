"use client";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { useChannelChatDraft } from "@/components/chat-mode/use-channel-chat-draft";

import { use, useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Check } from "lucide-react";
import { ExposureIcon } from "@/components/icons/facet-icons";
import { useAgent } from "@/hooks/use-agents";
import { useCreateAgentChannel, useSlackInstallCapability } from "@/hooks/use-agent-channels";
import { usePolicies } from "@/hooks/use-policies";
import { Button } from "@/components/ui/button";
import { ResourceNotFound } from "@/components/resource-not-found";
import {
  buildChannelConfig,
  ChannelForm,
  CHANNEL_FORM_KINDS,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/agents/channels/channel-form";
import {
  ChannelNextSteps,
  ChannelTypeChosen,
  ChannelTypePicker,
} from "@/components/agents/channels/channel-type-picker";
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
import { ChannelIcon } from "@/components/agents/channels/channel-icon";
import { getChannelTypeDisplayName } from "@/lib/channel-display";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";
import { beginSlackInstall } from "@/lib/api/agent-channels";

export default function NewAgentChannelPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { agentId } = use(params);
  const router = useRouter();
  const { data: agent, isLoading } = useAgent(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const createChannel = useCreateAgentChannel(agentId);
  const slackInstallCapability = useSlackInstallCapability();
  // `?kind=` preselects the type, e.g. the New agent page sending a Slack draft here.
  // Without it nothing is picked: the page opens on the choice, not on a form
  // for a type nobody asked for.
  const requestedKind = useSearchParams().get("kind");
  const preselected = CHANNEL_FORM_KINDS.find((kind) => kind === requestedKind);
  const [chosen, setChosen] = useState(!!preselected);
  const [formState, setFormState] = useState(() =>
    getDefaultChannelFormState(preselected ?? "webhook"),
  );
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);

  const chatDraft = useChannelChatDraft({
    state: formState,
    setState: setFormState,
    baseline: "new",
    available: !!agent,
    writable: !!agent && canManage && !createChannel.isPending,
  });
  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (isLoading || policiesLoading || slackInstallCapability.isLoading) {
    return <div className="container mx-auto p-6">{ui("Loading channel form...")}</div>;
  }
  if (!agent) {
    return (
      <ResourceNotFound
        title={ui("Agent not found")}
        description={ui(
          "This agent may have been deleted, moved to another organization, or the URL may be wrong.",
        )}
        backHref="/agents"
        backLabel="Back to agents"
        resourceId={agentId}
      />
    );
  }

  const agentName = getDisplayName(agent, displayLocale);
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
      {chatDraft.notice}
      <PageBreadcrumb
        items={[
          { label: ui("Agents"), href: "/agents" },
          { label: agentName, href: `/agents/${agentId}` },
          { label: ui("Channels"), href: returnHref },
          { label: ui("New") },
        ]}
      />
      <PageMasthead
        icon={
          chosen ? <ChannelIcon kind={formState.kind} className="icon-sharp" /> : <ExposureIcon />
        }
        title={
          chosen
            ? ui("New {value0}", { value0: getChannelTypeDisplayName(formState.kind) })
            : ui("New channel")
        }
        description={
          chosen
            ? ui(
                "It starts as a draft, so nobody can reach {value0} through it until you publish.",
                {
                  value0: agentName,
                },
              )
            : ui(
                "Pick where people or software will reach {value0}. You can add more channels later.",
                {
                  value0: agentName,
                },
              )
        }
        actions={
          <>
            {chosen && (
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
                {createChannel.isPending ? ui("Creating...") : ui("Create channel")}
              </Button>
            )}
            <Button type="button" variant="outline" onClick={() => router.push(returnHref)}>
              {ui("Cancel")}
            </Button>
          </>
        }
      />
      {!chosen ? (
        <ChannelTypePicker
          onChange={(kind) => {
            setFormState(getDefaultChannelFormState(kind));
            setChosen(true);
          }}
        />
      ) : (
        <form
          id="channel-edit-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (chatDraft.blocked) return;
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
                      caught instanceof Error
                        ? caught.message
                        : "Could not start the Slack install.";
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
              <div className="border bg-card">
                <ChannelTypeChosen kind={formState.kind} onChange={() => setChosen(false)} />
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="new"
                  slackInstallCapability={slackInstallCapability.data}
                  onSlackCapabilityChanged={slackInstallCapability.refetch}
                />
              </div>
            </PageMain>
            <PageRail>
              <ChannelNextSteps kind={formState.kind} />
            </PageRail>
          </PageColumns>
        </form>
      )}
      <PageFooter>
        <BackLink href={returnHref}>
          {ui("Back to")} {agentName}
        </BackLink>
      </PageFooter>
    </PageContainer>
  );
}
