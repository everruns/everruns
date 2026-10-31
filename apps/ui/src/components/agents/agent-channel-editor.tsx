"use client";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { useChannelChatDraft } from "@/components/chat-mode/use-channel-chat-draft";

import { useAgentSource } from "@/hooks/use-agent-source";
import { useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import {
  Check,
  CircleAlert,
  CirclePause,
  Copy,
  ExternalLink,
  MoreHorizontal,
  Pencil,
  Play,
  Trash2,
} from "lucide-react";
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
import { Button, buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { BudgetPanel } from "@/components/budgets/budget-panel";
import { ChannelIcon } from "@/components/agents/channels/channel-icon";
import {
  CHANNEL_STATUS_LABEL,
  channelDisplayName,
  channelFacts,
  channelStatus,
  lastUsedLabel,
  publicChatAccess,
  publicChatUrl,
} from "@/components/agents/channels/channel-summary";
import { ChannelDetailsPanel } from "@/components/agents/integrations/channel-details-panel";
import { ChannelUsePanel } from "@/components/agents/channel-use-panel";
import { SlackConnectionStatus } from "@/components/agents/integrations/slack-setup-guidance";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SlackRemovalNotice } from "@/components/agents/integrations/slack-removal-notice";
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
import { CallerDailyLimitField } from "@/components/agents/channels/api-fields";
import { WorkChatAddress, WorkChatHeaderLink } from "@/components/agents/channels/work-chat-link";
import { CronLabel } from "@/components/apps/cron-label";
import {
  BackLink,
  PageBreadcrumb,
  PageContainer,
  PageFooter,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import type { Agent, AgentChannel, ScheduleChannelConfig } from "@/lib/api/types";
import type { SlackInstallCapability } from "@/lib/api/agent-channels";
import {
  channelPublishControl,
  getChannelTypeDisplayName,
  getChannelLifecyclePresentation,
} from "@/lib/channel-display";
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
  const ui = useUiTranslations();
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { channels, isLoading: endpointsLoading } = useAgentChannels(agentId);
  const { can, isLoading: policiesLoading } = usePolicies("agents");
  const slackInstallCapability = useSlackInstallCapability();
  const sourceQuery = useAgentSource(agentId);
  const channel = channels.find(({ channel }) => channel.id === channelId)?.channel;
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const packageName = (channel?.channel_config as { package_name?: string })?.package_name;
  const sourceOwned =
    !!packageName &&
    sourceQuery.data?.mode !== "two_way" &&
    !!sourceQuery.data?.managed_channels.includes(packageName);
  const canManage = !policiesLoading && can("agent.manage") && !isReadOnlyStatus(agent?.status);
  const canDangerous =
    !policiesLoading && can("agent.dangerous") && !isReadOnlyStatus(agent?.status);

  useEffect(() => {
    if (agent && !policiesLoading && !canManage) router.replace(returnHref);
  }, [agent, canManage, policiesLoading, returnHref, router]);

  if (
    agentLoading ||
    endpointsLoading ||
    policiesLoading ||
    slackInstallCapability.isLoading ||
    sourceQuery.isLoading
  ) {
    return <div className="container mx-auto p-6">{ui("Loading channel...")}</div>;
  }
  if (!agent || !channel) {
    return (
      <ResourceNotFound
        title={ui("Channel not found")}
        description={ui(
          "This channel may have been deleted, moved to another agent, or the URL may be wrong.",
        )}
        backHref={returnHref}
        backLabel="Back to agent"
        resourceId={channelId}
      />
    );
  }

  if (sourceQuery.isError) {
    return (
      <div className="container mx-auto space-y-3 p-6">
        <p>{ui("Could not check the channel source. Reload before editing.")}</p>
        <BackLink href={returnHref}>{ui("Back to Agent")}</BackLink>
      </div>
    );
  }
  return (
    <AgentChannelForm
      key={channel.id}
      agent={agent}
      channel={channel}
      canManage={canManage}
      canDangerous={canDangerous}
      sourceOwned={sourceOwned}
      returnHref={returnHref}
      slackInstallCapability={slackInstallCapability.data}
      onSlackCapabilityChanged={slackInstallCapability.refetch}
      slackInstallFailure={slackInstallFailure}
    />
  );
}

type ChannelTab = "connect" | "settings";

/// The channel page. One layout reads and edits, like the agent page: Connect
/// shows how to reach the agent through this channel, Settings holds the form.
/// Work Chat has no separate connection, so it has no Connect tab. Its address
/// is shown once, above the settings.
/// Changing any setting turns the page into edit mode (Edit crumb, Editing
/// badge, Save changes and Discard); nothing saves until Save changes.
function AgentChannelForm({
  agent,
  channel,
  canManage,
  canDangerous,
  sourceOwned,
  returnHref,
  slackInstallCapability,
  onSlackCapabilityChanged,
  slackInstallFailure,
}: {
  agent: Agent;
  channel: AgentChannel;
  canManage: boolean;
  canDangerous: boolean;
  sourceOwned: boolean;
  returnHref: string;
  slackInstallCapability?: SlackInstallCapability;
  onSlackCapabilityChanged?: () => void | Promise<unknown>;
  slackInstallFailure?: string;
}) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const router = useRouter();
  const searchParams = useSearchParams();
  const agentId = agent.id;
  const channelId = channel.id;
  const updateEndpoint = useUpdateAgentChannel(agentId, channelId);
  const deleteEndpoint = useDeleteAgentChannel(agentId, channelId);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const publishEndpoint = usePublishAgentChannel(agentId);
  const triggerEndpoint = useTriggerAgentChannel(agentId);
  const { can: canBudget } = usePolicies("budgets");
  const [formState, setFormState] = useState<ChannelFormState>(() =>
    getDefaultChannelFormState(channel.channel_type, channel),
  );
  const chatDraft = useChannelChatDraft({
    state: formState,
    setState: setFormState,
    id: channelId,
    baseline: channel.updated_at,
    writable: canManage && !sourceOwned && !updateEndpoint.isPending,
  });
  // A Slack install failure is about setup, which lives under Settings.
  const requestedTab = searchParams.get("tab");
  const tab: ChannelTab =
    requestedTab === "settings" || (!requestedTab && slackInstallFailure !== undefined)
      ? "settings"
      : "connect";
  const selectTab = (next: ChannelTab) => {
    const params = new URLSearchParams(searchParams.toString());
    if (next === "connect") params.delete("tab");
    else params.set("tab", next);
    const query = params.toString();
    router.replace(`/agents/${agentId}/channels/${channelId}${query ? `?${query}` : ""}`, {
      scroll: false,
    });
  };

  const agentName = getDisplayName(agent, displayLocale);
  const name = channelDisplayName(channel);
  const lifecycle = getChannelLifecyclePresentation(channel);
  const status = channelStatus(channel);
  // The first fact is the type, which has its own row.
  const setupFacts = channelFacts(channel).slice(1);
  const publishHint = channelPublishControl(channel).hint;
  const slackInstallAvailable = slackInstallCapability?.connected === true;
  const slackInstallFailureMessage = slackInstallFailure
    ? /[.!?]$/.test(slackInstallFailure)
      ? slackInstallFailure
      : `${slackInstallFailure}.`
    : "Could not start the Slack install.";
  const schedule =
    channel.channel_type === "schedule" ? (channel.channel_config as ScheduleChannelConfig) : null;
  const workChat = channel.channel_type === "work_chat";
  const [hostedUrl, setHostedUrl] = useState<string | null>(null);
  const chatLink =
    channel.channel_type === "public_chat"
      ? publicChatUrl(channelId)
      : channel.channel_type === "work_chat"
        ? hostedUrl
        : null;

  // Saved settings, compared as the config the server would receive, so a
  // field returned to its saved value is not a change.
  const savedState = getDefaultChannelFormState(channel.channel_type, channel);
  const editing =
    formState.enabled !== savedState.enabled ||
    JSON.stringify(buildChannelConfig(formState)) !==
      JSON.stringify(buildChannelConfig(savedState));
  const discard = () => {
    setFormState(savedState);
    updateEndpoint.reset();
  };

  const saveChannel = () => {
    if (chatDraft.blocked || !canManage || !isChannelFormValid(formState)) return;
    updateEndpoint.mutate(
      {
        channel_config: buildChannelConfig(formState),
        enabled: formState.enabled,
      },
      {
        // Start again from what was saved: secret fields come back
        // blank, so the page leaves edit mode.
        onSuccess: (saved) =>
          setFormState(getDefaultChannelFormState(saved.channel_type, saved as AgentChannel)),
      },
    );
  };
  const publish = (next: boolean) => publishEndpoint.mutate({ channelId, publish: next });
  // Work Chat and Agent API cap each person as well as the shared pool. Both
  // sit in Spending, including on Connect, so Save writes the form state here.
  const callerSpend =
    !sourceOwned && (channel.channel_type === "api" || channel.channel_type === "work_chat");
  const primaryAction = editing ? null : !lifecycle.isLive ? (
    <Button
      type="button"
      variant="accent"
      title={publishHint}
      onClick={() => publish(true)}
      disabled={!canDangerous || publishEndpoint.isPending}
    >
      {publishEndpoint.isPending ? ui("Publishing...") : ui("Publish")}
    </Button>
  ) : chatLink ? (
    <a
      href={chatLink}
      target="_blank"
      rel="noopener noreferrer"
      className={buttonVariants({ variant: "accent" })}
    >
      <ExternalLink className="size-4" />
      {ui("Open chat page")}
    </a>
  ) : schedule ? (
    <Button
      type="button"
      variant="accent"
      onClick={() => triggerEndpoint.mutate(channelId)}
      disabled={!canManage || triggerEndpoint.isPending}
    >
      <Play className="size-4" />
      {ui("Run now")}
    </Button>
  ) : null;

  return (
    <PageContainer>
      {channel.channel_type === "work_chat" && (
        <WorkChatHeaderLink channelId={channelId} live={lifecycle.isLive} onUrl={setHostedUrl} />
      )}
      {chatDraft.notice}
      <PageBreadcrumb
        items={[
          { label: ui("Agents"), href: "/agents" },
          { label: agentName, href: `/agents/${agentId}` },
          { label: ui("Channels"), href: returnHref },
          editing
            ? { label: name, href: `/agents/${agentId}/channels/${channelId}` }
            : { label: name },
          ...(editing ? [{ label: ui("Edit") }] : []),
        ]}
      />
      <PageMasthead
        icon={<ChannelIcon kind={channel.channel_type} className="icon-sharp" />}
        title={ui(name)}
        entityId={channelId}
        badges={
          editing ? (
            <Badge variant="accent">
              <Pencil className="size-3" />
              {ui("Editing")}
            </Badge>
          ) : (
            <Badge variant={status === "published" ? "success" : "secondary"}>
              {ui(CHANNEL_STATUS_LABEL[status])}
            </Badge>
          )
        }
        description={
          editing
            ? lifecycle.isLive
              ? ui("This channel is published, so saving changes it for everyone using it now.")
              : ui("This channel is a draft. Saving does not open it; publish when you are ready.")
            : channelDescription(channel, agentName, ui)
        }
        meta={
          editing ? undefined : (
            <>
              <span>
                {channel.last_invoked_at
                  ? ui("Last used {value0}", { value0: lastUsedLabel(channel.last_invoked_at) })
                  : ui("Never used")}
              </span>
              {schedule && (
                <span>
                  <CronLabel expr={schedule.cron_expression} tz={schedule.timezone} />
                </span>
              )}
              <span>
                {ui("Created {value0}", {
                  value0: new Date(channel.created_at).toLocaleDateString(displayLocale),
                })}
              </span>
            </>
          )
        }
        actions={
          editing ? (
            <>
              {chatLink && (
                <a
                  href={chatLink}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={buttonVariants({ variant: "outline" })}
                >
                  <ExternalLink className="size-4" />
                  {ui("Open chat page")}
                </a>
              )}
              <Button
                type="button"
                onClick={saveChannel}
                disabled={
                  chatDraft.blocked ||
                  !canManage ||
                  !isChannelFormValid(formState) ||
                  updateEndpoint.isPending
                }
              >
                <Check className="size-4" />
                {updateEndpoint.isPending ? ui("Saving...") : ui("Save changes")}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={discard}
                disabled={updateEndpoint.isPending}
              >
                {ui("Discard")}
              </Button>
            </>
          ) : (
            <>
              <DropdownMenu>
                <DropdownMenuTrigger
                  className={buttonVariants({ variant: "outline", size: "icon" })}
                  aria-label={ui("More actions")}
                >
                  <MoreHorizontal className="size-4" />
                </DropdownMenuTrigger>
                <DropdownMenuPositioner align="end">
                  <DropdownMenuContent>
                    {chatLink && (
                      <DropdownMenuItem
                        onClick={() => void navigator.clipboard?.writeText(chatLink)}
                      >
                        <Copy className="size-4" />
                        {ui("Copy link")}
                      </DropdownMenuItem>
                    )}
                    {schedule && lifecycle.isLive && primaryAction === null && (
                      <DropdownMenuItem
                        onClick={() => triggerEndpoint.mutate(channelId)}
                        disabled={!canManage || triggerEndpoint.isPending}
                      >
                        <Play className="size-4" />
                        {ui("Run now")}
                      </DropdownMenuItem>
                    )}
                    {lifecycle.isLive && (
                      <DropdownMenuItem
                        onClick={() => publish(false)}
                        disabled={!canDangerous || publishEndpoint.isPending}
                      >
                        <CirclePause className="size-4" />
                        {ui("Unpublish")}
                      </DropdownMenuItem>
                    )}
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      variant="destructive"
                      onClick={() => setConfirmDelete(true)}
                      disabled={!canDangerous || deleteEndpoint.isPending}
                    >
                      <Trash2 className="size-4" />
                      {ui("Delete channel...")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenuPositioner>
              </DropdownMenu>
              {primaryAction}
            </>
          )
        }
      />
      {(publishEndpoint.error || triggerEndpoint.error) && (
        <p role="alert" className="text-sm text-destructive">
          {(publishEndpoint.error ?? triggerEndpoint.error)?.message}
        </p>
      )}
      {updateEndpoint.error && (
        <p role="alert" className="text-sm text-destructive">
          {ui("Could not save:")} {updateEndpoint.error.message}
        </p>
      )}

      {/* The tab row and the workspace are one surface: no gap between them. */}
      <div className="flex min-w-0 flex-col">
        {!workChat && (
          <SectionTabs
            value={tab}
            onValueChange={(value) => selectTab(value as ChannelTab)}
            aria-label={ui("Channel sections")}
            items={[
              { value: "connect", label: ui("Connect") },
              { value: "settings", label: editing ? ui("Settings (edited)") : ui("Settings") },
            ]}
            className="border-x border-t bg-background px-2"
          />
        )}
        <div
          className={`grid min-w-0 border-x border-b bg-card lg:grid-cols-[minmax(0,1fr)_340px] ${workChat ? "border-t" : ""}`}
        >
          <div className="min-w-0 lg:border-r">
            {workChat ? (
              <>
                {!lifecycle.isLive && (
                  <div className="p-5 pb-0">
                    <Notice icon={<CircleAlert className="size-4" />}>
                      <NoticeTitle>{ui("Draft")}</NoticeTitle>
                      <NoticeDescription>
                        {ui(
                          "Nobody can reach the agent through this channel yet. Publish when you are ready.",
                        )}
                      </NoticeDescription>
                    </Notice>
                  </div>
                )}
                <div className="p-5">
                  <WorkChatAddress channel={channel} />
                </div>
                {sourceOwned ? (
                  <div className="border-t p-5">
                    <Notice icon={<CircleAlert className="size-4" />}>
                      <NoticeTitle>{ui("Managed in GitHub")}</NoticeTitle>
                      <NoticeDescription>
                        {ui(
                          "This channel's settings come from the agent's source repository. Change them there. Publishing and keys still work here.",
                        )}
                      </NoticeDescription>
                    </Notice>
                  </div>
                ) : (
                  <form
                    id="channel-edit-form"
                    className="border-t"
                    onSubmit={(event) => {
                      event.preventDefault();
                      saveChannel();
                    }}
                  >
                    <ChannelForm
                      state={formState}
                      onChange={setFormState}
                      mode="edit"
                      channelId={channel.id}
                      channel={channel}
                      slackInstallDisabled={editing}
                      slackInstallCapability={slackInstallCapability}
                      onSlackCapabilityChanged={onSlackCapabilityChanged}
                    />
                  </form>
                )}
              </>
            ) : tab === "connect" ? (
              <div className="flex flex-col gap-6 p-5">
                {!lifecycle.isLive && (
                  <Notice icon={<CircleAlert className="size-4" />}>
                    <NoticeTitle>{ui("Draft")}</NoticeTitle>
                    <NoticeDescription>
                      {ui(
                        "Nobody can reach the agent through this channel yet. Publish when you are ready.",
                      )}
                    </NoticeDescription>
                  </Notice>
                )}
                {channel.channel_type === "slack" && (
                  <>
                    <ChannelHealthWarning channelId={channel.id} />
                    <SlackConnectionStatus channel={channel} />
                    <ChannelUsePanel channel={channel} agentId={agentId} />
                  </>
                )}
                {channel.channel_type !== "slack" && (
                  <ChannelDetailsPanel
                    agentId={agentId}
                    agentName={agentName}
                    agentDescription={agent.description}
                    channel={channel}
                    configureHref={
                      canManage && !sourceOwned
                        ? `/agents/${agentId}/channels/${channelId}?tab=settings`
                        : undefined
                    }
                  />
                )}
                {channel.channel_type === "api" && (
                  <AgentKeysCard agentId={agentId} channelId={channelId} canManage={canManage} />
                )}
              </div>
            ) : sourceOwned ? (
              <div className="p-5">
                <Notice icon={<CircleAlert className="size-4" />}>
                  <NoticeTitle>{ui("Managed in GitHub")}</NoticeTitle>
                  <NoticeDescription>
                    {ui(
                      "This channel's settings come from the agent's source repository. Change them there. Publishing and keys still work here.",
                    )}
                  </NoticeDescription>
                </Notice>
              </div>
            ) : (
              <form
                id="channel-edit-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  saveChannel();
                }}
              >
                {slackInstallFailure !== undefined && (
                  <div className="p-5 pb-0">
                    <Notice
                      variant="destructive"
                      icon={<CircleAlert className="size-4" />}
                      role="alert"
                    >
                      <NoticeTitle>{ui("Slack install did not start")}</NoticeTitle>
                      <NoticeDescription>
                        {ui("The channel was saved.")} {slackInstallFailureMessage}{" "}
                        {slackInstallAvailable
                          ? ui("Use Add to Slack to try again, or configure Slack manually.")
                          : ui("Configure Slack manually.")}
                      </NoticeDescription>
                    </Notice>
                  </div>
                )}
                <ChannelForm
                  state={formState}
                  onChange={setFormState}
                  mode="edit"
                  channelId={channel.id}
                  channel={channel}
                  slackInstallDisabled={editing}
                  slackInstallCapability={slackInstallCapability}
                  onSlackCapabilityChanged={onSlackCapabilityChanged}
                />
              </form>
            )}
          </div>
          <aside className="flex min-w-0 flex-col gap-4 bg-muted/30 p-5 text-sm">
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2">
              <dt className="text-muted-foreground">{ui("Status")}</dt>
              <dd className="text-right">{ui(CHANNEL_STATUS_LABEL[status])}</dd>
              <dt className="text-muted-foreground">{ui("Type")}</dt>
              <dd className="text-right">{getChannelTypeDisplayName(channel.channel_type)}</dd>
              {setupFacts.length > 0 && (
                <>
                  <dt className="text-muted-foreground">{ui("Setup")}</dt>
                  <dd className="text-right">
                    {setupFacts.map((fact) => (
                      <span key={fact} className="block">
                        {ui(fact)}
                      </span>
                    ))}
                  </dd>
                </>
              )}
            </dl>
            {!workChat && tab === "connect" && canManage && !sourceOwned && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="self-start"
                onClick={() => selectTab("settings")}
              >
                <Pencil className="size-3.5" />
                {ui("Change settings")}
              </Button>
            )}
            <p className="text-xs text-muted-foreground">
              {ui(
                "Publish opens this channel to callers and Unpublish closes it. Pausing all channels on the agent's Integrations tab overrides both.",
              )}
            </p>
            {(canBudget("budget.view") || callerSpend) && (
              <div className="space-y-4 border-t pt-4">
                <div>
                  <h3 className="text-sm font-semibold">{ui("Spending")}</h3>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {callerSpend
                      ? ui(
                          "The shared pool covers everyone on this channel. The daily limit caps each person until midnight UTC.",
                        )
                      : ui(
                          "Caps what this channel can spend, separately from the agent's budget.",
                        )}
                  </p>
                </div>
                {canBudget("budget.view") && (
                  <BudgetPanel
                    subjectType="agent_channel"
                    subjectId={channelId}
                    title={ui("Shared pool")}
                    canManage={canBudget("budget.manage")}
                  />
                )}
                {callerSpend && (
                  <CallerDailyLimitField
                    value={formState.api.dailySpendLimitUsd}
                    disabled={!canManage || updateEndpoint.isPending}
                    onChange={(dailySpendLimitUsd) =>
                      setFormState((current) => ({
                        ...current,
                        api: { ...current.api, dailySpendLimitUsd },
                      }))
                    }
                  />
                )}
              </div>
            )}
          </aside>
        </div>
      </div>
      <PageFooter>
        <BackLink href={returnHref}>
          {ui("Back to")} {agentName}
        </BackLink>
      </PageFooter>
      <Dialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{ui("Delete {value0}?", { value0: name })}</DialogTitle>
            <DialogDescription>
              {ui(
                "The channel closes at once and {value0} can no longer be reached through it. Past conversations stay in Sessions.",
                { value0: agentName },
              )}
            </DialogDescription>
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
              {ui("Cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={!canDangerous || deleteEndpoint.isPending}
              onClick={() =>
                deleteEndpoint.mutate(undefined, { onSuccess: () => router.push(returnHref) })
              }
            >
              {deleteEndpoint.isPending ? ui("Deleting...") : ui("Delete channel")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}

/// One plain line under the channel's name: what it is and who reaches it.
function channelDescription(
  channel: AgentChannel,
  agentName: string,
  ui: (message: string, values?: Record<string, unknown>) => string,
): string {
  const type = getChannelTypeDisplayName(channel.channel_type);
  if (channel.channel_type === "public_chat") {
    return ui("{value0} for {value1}. {value2}.", {
      value0: type,
      value1: agentName,
      value2: ui(publicChatAccess(channel)),
    });
  }
  return ui("{value0} for {value1}.", { value0: type, value1: agentName });
}
