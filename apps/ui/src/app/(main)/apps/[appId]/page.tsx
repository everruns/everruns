"use client";

import { use, useState, useCallback } from "react";
import {
  useApp,
  useUpdateApp,
  useDeleteApp,
  useDestroyApp,
  usePublishApp,
  useUnpublishApp,
} from "@/hooks/use-apps";
import { usePolicies } from "@/hooks/use-policies";
import { useAgents } from "@/hooks";
import { useHarnesses } from "@/hooks/use-harnesses";
import {
  getSlackManifest,
  updateChannel as apiUpdateChannel,
  addChannel as apiAddChannel,
  deleteChannel as apiDeleteChannel,
} from "@/lib/api/apps";
import { useRouter } from "next/navigation";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import Link from "next/link";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AgentSelect } from "@/components/agent/agent-select";
import { AgentIdentitySelect } from "@/components/agent-identity/agent-identity-select";
import { HarnessSelect } from "@/components/harness/harness-select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ArrowLeft, Globe, GlobeLock, Trash2, Pencil, Check, X, Rocket, Plus } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import { AgUiSetupGuidance } from "@/components/apps/ag-ui-setup-guidance";
import { AppBudgetsCard } from "@/components/apps/app-budgets-card";
import { ScheduleSetupGuidance } from "@/components/apps/schedule-setup-guidance";
import { SlackSetupGuidance } from "@/components/apps/slack-setup-guidance";
import { WebhookSetupGuidance } from "@/components/apps/webhook-setup-guidance";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import type {
  AgUiChannelConfig,
  AppChannel,
  ChannelType,
  InvocationSessionMode,
  ScheduleChannelConfig,
  SessionStrategy,
  SlackChannelConfig,
  SlackReplyMode,
  WebhookChannelConfig,
} from "@/lib/api/types";
import { DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS } from "@/lib/api/types/app-types";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { getChannelTypeDisplayName, getInvocationSessionModeDisplayName } from "@/lib/app-channels";

type ChannelConfigInput =
  | SlackChannelConfig
  | AgUiChannelConfig
  | ScheduleChannelConfig
  | WebhookChannelConfig;

export default function AppDetailPage({ params }: { params: Promise<{ appId: string }> }) {
  const { appId } = use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data: app, isLoading } = useApp(appId);
  const { data: agents } = useAgents({ includeArchived: true });
  const { data: harnesses } = useHarnesses({ includeArchived: true });
  const updateApp = useUpdateApp();
  const deleteAppMutation = useDeleteApp();
  const destroyAppMutation = useDestroyApp();
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();
  const { can: canPolicies } = usePolicies("apps");
  const appBudgetsEnabled = useFeatureFlag("app_budgets");

  const [editingBasic, setEditingBasic] = useState(false);
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [showAddChannel, setShowAddChannel] = useState(false);
  const [editingChannelType, setEditingChannelType] = useState<ChannelType>("slack");
  const [addChannelType, setAddChannelType] = useState<ChannelType>("slack");

  // Basic info edit state
  const [editName, setEditName] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editAgentId, setEditAgentId] = useState("");
  const [editHarnessId, setEditHarnessId] = useState("");
  const [editAgentIdentityId, setEditAgentIdentityId] = useState("");

  // Channel config edit state (shared for edit and add)
  const [editSigningSecret, setEditSigningSecret] = useState("");
  const [editBotToken, setEditBotToken] = useState("");
  const [editChannelIdField, setEditChannelIdField] = useState("");
  const [editTeamId, setEditTeamId] = useState("");
  const [editSessionStrategy, setEditSessionStrategy] = useState<SessionStrategy>("per_thread");
  const [editReplyMode, setEditReplyMode] = useState<SlackReplyMode>("all_messages");
  const [editScheduleCronExpression, setEditScheduleCronExpression] = useState("0 * * * * * *");
  const [editScheduleTimezone, setEditScheduleTimezone] = useState("UTC");
  const [editInvocationSessionMode, setEditInvocationSessionMode] =
    useState<InvocationSessionMode>("shared_session");
  const [editChannelMessage, setEditChannelMessage] = useState("");
  const [editWebhookToken, setEditWebhookToken] = useState("");
  const [editAgUiExpirationHours, setEditAgUiExpirationHours] = useState(
    DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600,
  );
  const [editChannelEnabled, setEditChannelEnabled] = useState(true);
  const [editAgUiRateLimitPerMinute, setEditAgUiRateLimitPerMinute] = useState<string>("");

  const [creatingSlackApp, setCreatingSlackApp] = useState(false);

  const invalidateApp = () => {
    queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
    queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
  };

  const updateChannelMutation = useMutation({
    mutationFn: async ({
      channelId,
      config,
      enabled,
    }: {
      channelId: string;
      config: ChannelConfigInput;
      enabled: boolean;
    }) => {
      return apiUpdateChannel(appId, channelId, { channel_config: config, enabled });
    },
    onSuccess: invalidateApp,
  });

  const addChannelMutation = useMutation({
    mutationFn: async ({
      channelType,
      config,
      enabled,
    }: {
      channelType: ChannelType;
      config: ChannelConfigInput;
      enabled: boolean;
    }) => {
      return apiAddChannel(appId, {
        channel_type: channelType,
        channel_config: config,
        enabled,
      });
    },
    onSuccess: () => {
      invalidateApp();
      setShowAddChannel(false);
    },
  });

  const deleteChannelMutation = useMutation({
    mutationFn: async (channelId: string) => {
      return apiDeleteChannel(appId, channelId);
    },
    onSuccess: invalidateApp,
  });

  const isPublished = app?.status === "published";
  const isArchived = app?.status === "archived";
  const isReadOnly = isReadOnlyStatus(app?.status);
  const canDangerousDelete = canPolicies("app.dangerous");

  // Find first Slack channel for backward-compat checks
  const slackChannel = app?.channels?.find(
    (ch: AppChannel) => ch.channel_type === "slack" && ch.enabled,
  );
  const slackConfig = slackChannel?.channel_config as SlackChannelConfig | undefined;
  const hasSlackConfig = slackConfig?.signing_secret && slackConfig?.bot_token;
  const canPublishApp =
    app?.channels?.some((channel) => {
      if (!channel.enabled) {
        return false;
      }
      switch (channel.channel_type) {
        case "slack": {
          const config = channel.channel_config as SlackChannelConfig;
          return !!config?.signing_secret && !!config?.bot_token;
        }
        case "ag_ui":
          return true;
        case "schedule": {
          const config = channel.channel_config as ScheduleChannelConfig;
          return !!config?.cron_expression && !!config?.message;
        }
        case "webhook": {
          const config = channel.channel_config as WebhookChannelConfig;
          return !!config?.token && !!config?.message;
        }
      }
    }) ?? false;

  const slackWebhookUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${appId}/slack/events`
      : `/api/v1/apps/${appId}/slack/events`;
  const agUiEndpointUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${appId}/ag-ui`
      : `/api/v1/apps/${appId}/ag-ui`;

  const isLocalhost =
    typeof window !== "undefined" &&
    (window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1");
  const slackWebhookPath = `/api/v1/apps/${appId}/slack/events`;

  const handleCreateSlackApp = useCallback(async () => {
    setCreatingSlackApp(true);
    try {
      const manifest = await getSlackManifest(appId);
      if (manifest?.create_url) {
        window.open(manifest.create_url, "_blank");
      }
    } finally {
      setCreatingSlackApp(false);
    }
  }, [appId]);

  const startEditBasic = () => {
    if (!app) return;
    setEditName(app.name);
    setEditDescription(app.description ?? "");
    setEditAgentId(app.agent_id ?? "");
    setEditHarnessId(app.harness_id);
    setEditAgentIdentityId(app.agent_identity_id ?? "");
    setEditingBasic(true);
  };

  const saveBasic = async () => {
    if (!app) return;
    await updateApp.mutateAsync({
      appId: app.id,
      data: {
        name: editName,
        description: editDescription || undefined,
        agent_id: editAgentId || undefined,
        agent_identity_id: editAgentIdentityId || null,
        harness_id: editHarnessId,
      },
    });
    setEditingBasic(false);
  };

  const resetChannelForm = (channelType: ChannelType) => {
    setEditingChannelType(channelType);
    setAddChannelType(channelType);
    setEditSigningSecret("");
    setEditBotToken("");
    setEditChannelIdField("");
    setEditTeamId("");
    setEditSessionStrategy("per_thread");
    setEditReplyMode("all_messages");
    setEditScheduleCronExpression("0 * * * * * *");
    setEditScheduleTimezone("UTC");
    setEditInvocationSessionMode("shared_session");
    setEditChannelMessage("");
    setEditWebhookToken("");
    setEditAgUiExpirationHours(DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600);
    setEditChannelEnabled(true);
    setEditAgUiRateLimitPerMinute("");
  };

  const buildChannelConfig = (channelType: ChannelType): ChannelConfigInput => {
    switch (channelType) {
      case "ag_ui": {
        const hours = Number.isFinite(editAgUiExpirationHours)
          ? Math.max(0, editAgUiExpirationHours)
          : DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS / 3600;
        const trimmed = editAgUiRateLimitPerMinute.trim();
        const parsed = trimmed === "" ? undefined : Number.parseInt(trimmed, 10);
        return {
          anonymous: true,
          session_expiration_seconds: Math.round(hours * 3600),
          ...(parsed !== undefined && Number.isFinite(parsed) && parsed > 0
            ? { rate_limit_per_minute: parsed }
            : {}),
        };
      }
      case "schedule":
        return {
          cron_expression: editScheduleCronExpression,
          timezone: editScheduleTimezone || "UTC",
          session_mode: editInvocationSessionMode,
          message: editChannelMessage,
        };
      case "webhook":
        return {
          token: editWebhookToken,
          session_mode: editInvocationSessionMode,
          message: editChannelMessage,
        };
      case "slack":
        return {
          signing_secret: editSigningSecret,
          bot_token: editBotToken,
          session_strategy: editSessionStrategy,
          reply_mode: editReplyMode,
          ...(editChannelIdField ? { channel_id: editChannelIdField } : {}),
          ...(editTeamId ? { team_id: editTeamId } : {}),
        };
    }
  };

  const isChannelConfigValid = (channelType: ChannelType) => {
    switch (channelType) {
      case "ag_ui": {
        if (!Number.isFinite(editAgUiExpirationHours) || editAgUiExpirationHours < 0) {
          return false;
        }
        const trimmed = editAgUiRateLimitPerMinute.trim();
        if (trimmed === "") return true;
        const parsed = Number.parseInt(trimmed, 10);
        return (
          Number.isFinite(parsed) &&
          parsed >= 0 &&
          parsed <= 1_000_000 &&
          String(parsed) === trimmed
        );
      }
      case "schedule":
        return !!editScheduleCronExpression && !!editChannelMessage;
      case "webhook":
        return !!editWebhookToken && !!editChannelMessage;
      case "slack":
        return !!editSigningSecret && !!editBotToken;
    }
  };

  const startEditChannel = (channel: AppChannel) => {
    resetChannelForm(channel.channel_type);
    setEditChannelEnabled(channel.enabled);
    if (channel.channel_type === "ag_ui") {
      const config = channel.channel_config as AgUiChannelConfig;
      const expSeconds =
        config?.session_expiration_seconds ?? DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS;
      setEditAgUiExpirationHours(expSeconds / 3600);
      setEditAgUiRateLimitPerMinute(
        config?.rate_limit_per_minute && config.rate_limit_per_minute > 0
          ? String(config.rate_limit_per_minute)
          : "",
      );
    } else if (channel.channel_type === "slack") {
      const config = channel.channel_config as SlackChannelConfig;
      setEditSigningSecret(config?.signing_secret ?? "");
      setEditBotToken(config?.bot_token ?? "");
      setEditChannelIdField(config?.channel_id ?? "");
      setEditTeamId(config?.team_id ?? "");
      setEditSessionStrategy(config?.session_strategy ?? "per_thread");
      setEditReplyMode(config?.reply_mode ?? "all_messages");
    } else if (channel.channel_type === "schedule") {
      const config = channel.channel_config as ScheduleChannelConfig;
      setEditScheduleCronExpression(config?.cron_expression ?? "0 * * * * * *");
      setEditScheduleTimezone(config?.timezone ?? "UTC");
      setEditInvocationSessionMode(config?.session_mode ?? "shared_session");
      setEditChannelMessage(config?.message ?? "");
    } else if (channel.channel_type === "webhook") {
      const config = channel.channel_config as WebhookChannelConfig;
      setEditWebhookToken(config?.token ?? "");
      setEditInvocationSessionMode(config?.session_mode ?? "shared_session");
      setEditChannelMessage(config?.message ?? "");
    }
    setEditingChannelId(channel.id);
  };

  const startAddChannel = () => {
    resetChannelForm("slack");
    setShowAddChannel(true);
  };

  const saveChannel = async () => {
    if (!editingChannelId) return;
    await updateChannelMutation.mutateAsync({
      channelId: editingChannelId,
      config: buildChannelConfig(editingChannelType),
      enabled: editChannelEnabled,
    });
    setEditingChannelId(null);
  };

  const saveNewChannel = async () => {
    await addChannelMutation.mutateAsync({
      channelType: addChannelType,
      config: buildChannelConfig(addChannelType),
      enabled: editChannelEnabled,
    });
  };

  const handleDelete = async () => {
    if (!app) return;
    await destroyAppMutation.mutateAsync(app.id);
    router.push("/apps");
  };

  const handleArchive = async () => {
    if (!app) return;
    await deleteAppMutation.mutateAsync(app.id);
  };

  const agent = agents?.find((candidate) => candidate.id === app?.agent_id);
  const harness = harnesses?.find((candidate) => candidate.id === app?.harness_id);

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
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

  const renderChannelForm = (isSaving: boolean, formId: string = "default") => {
    const formChannelType = editingChannelId ? editingChannelType : addChannelType;

    return (
      <div className="space-y-4">
        {!editingChannelId && (
          <div>
            <Label htmlFor={`channel_type_${formId}`}>Channel Type</Label>
            <Select
              value={addChannelType}
              onValueChange={(v) => resetChannelForm(v as ChannelType)}
            >
              <SelectTrigger id={`channel_type_${formId}`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="slack">Slack</SelectItem>
                <SelectItem value="ag_ui">AG-UI</SelectItem>
                <SelectItem value="schedule">Schedule</SelectItem>
                <SelectItem value="webhook">Webhook</SelectItem>
              </SelectContent>
            </Select>
          </div>
        )}

        <div className="flex items-center justify-between rounded-md border p-3">
          <div className="space-y-1">
            <p className="text-sm font-medium">Enabled</p>
            <p className="text-xs text-muted-foreground">
              Disabled channels stay attached to the app but do not accept or emit new invocations.
            </p>
          </div>
          <Switch checked={editChannelEnabled} onCheckedChange={setEditChannelEnabled} />
        </div>

        {formChannelType === "ag_ui" && (
          <>
            <div className="rounded-md border p-3 text-sm text-muted-foreground">
              AG-UI requests are accepted anonymously for now. Publish the app, then point an AG-UI
              client at the endpoint shown on this page.
            </div>

            <div>
              <Label htmlFor={`ag_ui_expiration_${formId}`}>Thread expiration (hours)</Label>
              <Input
                id={`ag_ui_expiration_${formId}`}
                type="number"
                min="0"
                step="0.25"
                value={editAgUiExpirationHours}
                onChange={(e) => setEditAgUiExpirationHours(Number(e.target.value))}
                placeholder="6"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                After this window, requests reusing the same `threadId` are rejected with{" "}
                <code>410 Gone</code> and the client must start a new thread. Set to `0` to allow
                resumption indefinitely. Default is 6 hours.
              </p>
            </div>

            <div>
              <Label htmlFor={`ag_ui_rate_limit_${formId}`}>
                Rate limit (requests per minute, per IP)
              </Label>
              <Input
                id={`ag_ui_rate_limit_${formId}`}
                type="number"
                min={0}
                max={1000000}
                step={1}
                value={editAgUiRateLimitPerMinute}
                onChange={(e) => setEditAgUiRateLimitPerMinute(e.target.value)}
                placeholder="Leave blank to disable per-app cap"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Throttles anonymous traffic to this app&apos;s AG-UI endpoint. Leave blank or set to
                0 to rely on the global API rate limit only. Maximum 1,000,000 per minute.
              </p>
            </div>
          </>
        )}

        {formChannelType === "slack" && (
          <>
            <div>
              <Label htmlFor={`signing_secret_${formId}`}>Signing Secret</Label>
              <Input
                id={`signing_secret_${formId}`}
                type="password"
                value={editSigningSecret}
                onChange={(e) => setEditSigningSecret(e.target.value)}
                placeholder="Your Slack app's signing secret"
                required
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Found in your Slack app &rarr; Settings &rarr; Basic Information &rarr; App
                Credentials
              </p>
            </div>

            <div>
              <Label htmlFor={`bot_token_${formId}`}>Bot User OAuth Token</Label>
              <Input
                id={`bot_token_${formId}`}
                type="password"
                value={editBotToken}
                onChange={(e) => setEditBotToken(e.target.value)}
                placeholder="xoxb-..."
                required
              />
              <p className="mt-1 text-xs text-muted-foreground">
                Found in your Slack app &rarr; OAuth &amp; Permissions &rarr; Bot User OAuth Token
              </p>
            </div>

            <Separator />

            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor={`team_id_${formId}`}>Workspace ID (optional)</Label>
                <Input
                  id={`team_id_${formId}`}
                  value={editTeamId}
                  onChange={(e) => setEditTeamId(e.target.value)}
                  placeholder="T0123456789"
                />
              </div>

              <div>
                <Label htmlFor={`channel_id_${formId}`}>Channel ID (optional)</Label>
                <Input
                  id={`channel_id_${formId}`}
                  value={editChannelIdField}
                  onChange={(e) => setEditChannelIdField(e.target.value)}
                  placeholder="C0123456789"
                />
              </div>
            </div>

            <div>
              <Label htmlFor={`session_strategy_${formId}`}>Session Strategy</Label>
              <Select
                value={editSessionStrategy}
                onValueChange={(v) => setEditSessionStrategy(v as SessionStrategy)}
              >
                <SelectTrigger id={`session_strategy_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="per_thread">Per Thread (default)</SelectItem>
                  <SelectItem value="per_channel">Per Channel</SelectItem>
                  <SelectItem value="per_user">Per User</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div>
              <Label htmlFor={`reply_mode_${formId}`}>Reply Mode</Label>
              <Select
                value={editReplyMode}
                onValueChange={(v) => setEditReplyMode(v as SlackReplyMode)}
              >
                <SelectTrigger id={`reply_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all_messages">All Assistant Messages</SelectItem>
                  <SelectItem value="report_progress_only">Report Progress Only</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </>
        )}

        {formChannelType === "schedule" && (
          <>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor={`schedule_cron_${formId}`}>Cron Expression</Label>
                <Input
                  id={`schedule_cron_${formId}`}
                  value={editScheduleCronExpression}
                  onChange={(e) => setEditScheduleCronExpression(e.target.value)}
                  placeholder="0 * * * * * *"
                />
              </div>
              <div>
                <Label htmlFor={`schedule_timezone_${formId}`}>Timezone</Label>
                <Input
                  id={`schedule_timezone_${formId}`}
                  value={editScheduleTimezone}
                  onChange={(e) => setEditScheduleTimezone(e.target.value)}
                  placeholder="UTC"
                />
              </div>
            </div>
            <div>
              <Label htmlFor={`schedule_session_mode_${formId}`}>Invocation Session Mode</Label>
              <Select
                value={editInvocationSessionMode}
                onValueChange={(v) => setEditInvocationSessionMode(v as InvocationSessionMode)}
              >
                <SelectTrigger id={`schedule_session_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor={`schedule_message_${formId}`}>Invocation Message</Label>
              <Textarea
                id={`schedule_message_${formId}`}
                value={editChannelMessage}
                onChange={(e) => setEditChannelMessage(e.target.value)}
                placeholder="Run repository checks for {{app.name}}"
              />
            </div>
          </>
        )}

        {formChannelType === "webhook" && (
          <>
            <div>
              <Label htmlFor={`webhook_token_${formId}`}>Webhook Token</Label>
              <Input
                id={`webhook_token_${formId}`}
                type="password"
                value={editWebhookToken}
                onChange={(e) => setEditWebhookToken(e.target.value)}
                placeholder="shared-secret"
              />
            </div>
            <div>
              <Label htmlFor={`webhook_session_mode_${formId}`}>Invocation Session Mode</Label>
              <Select
                value={editInvocationSessionMode}
                onValueChange={(v) => setEditInvocationSessionMode(v as InvocationSessionMode)}
              >
                <SelectTrigger id={`webhook_session_mode_${formId}`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared_session">
                    {getInvocationSessionModeDisplayName("shared_session")}
                  </SelectItem>
                  <SelectItem value="session_per_invocation">
                    {getInvocationSessionModeDisplayName("session_per_invocation")}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label htmlFor={`webhook_message_${formId}`}>Invocation Message</Label>
              <Textarea
                id={`webhook_message_${formId}`}
                value={editChannelMessage}
                onChange={(e) => setEditChannelMessage(e.target.value)}
                placeholder="Process webhook payload for {{payload.repo.name}}"
              />
            </div>
          </>
        )}

        <div className="flex gap-2 pt-2">
          <Button
            size="sm"
            onClick={editingChannelId ? saveChannel : saveNewChannel}
            disabled={isSaving || !isChannelConfigValid(formChannelType)}
          >
            <Check className="w-3 h-3 mr-1" />
            {isSaving ? "Saving..." : "Save"}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setEditingChannelId(null);
              setShowAddChannel(false);
            }}
          >
            <X className="w-3 h-3 mr-1" />
            Cancel
          </Button>
        </div>
      </div>
    );
  };

  const renderChannelDisplay = (channel: AppChannel) => {
    if (channel.channel_type === "ag_ui") {
      const config = channel.channel_config as AgUiChannelConfig;
      return (
        <div key={channel.id} className="space-y-4">
          <AgUiSetupGuidance
            endpointUrl={agUiEndpointUrl}
            isPublished={isPublished}
            anonymousEnabled={config?.anonymous ?? true}
            sessionExpirationSeconds={
              config?.session_expiration_seconds ?? DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS
            }
            rateLimitPerMinute={config?.rate_limit_per_minute}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    if (channel.channel_type === "schedule") {
      const config = channel.channel_config as ScheduleChannelConfig;
      return (
        <div key={channel.id} className="space-y-4">
          <ScheduleSetupGuidance
            cronExpression={config?.cron_expression ?? ""}
            timezone={config?.timezone ?? "UTC"}
            sessionMode={config?.session_mode ?? "shared_session"}
            message={config?.message ?? ""}
            isPublished={isPublished}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    if (channel.channel_type === "webhook") {
      const config = channel.channel_config as WebhookChannelConfig;
      const endpointUrl =
        typeof window !== "undefined"
          ? `${window.location.origin}/api/v1/apps/${appId}/webhooks/${channel.id}`
          : `/api/v1/apps/${appId}/webhooks/${channel.id}`;

      return (
        <div key={channel.id} className="space-y-4">
          <WebhookSetupGuidance
            endpointUrl={endpointUrl}
            sessionMode={config?.session_mode ?? "shared_session"}
            message={config?.message ?? ""}
            tokenConfigured={!!config?.token}
            isPublished={isPublished}
            onConfigure={() => startEditChannel(channel)}
          />
        </div>
      );
    }

    const config = channel.channel_config as SlackChannelConfig;
    const chHasConfig = config?.signing_secret && config?.bot_token;
    const chWebhookVerified = !!config?.webhook_verified_at;
    const chFirstMsg = !!config?.first_message_received_at;

    return (
      <div key={channel.id} className="space-y-4">
        {chHasConfig ? (
          <>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <p className="text-sm font-medium">Signing Secret</p>
                <p className="text-sm text-muted-foreground font-mono">{"*".repeat(12)}</p>
              </div>
              <div>
                <p className="text-sm font-medium">Bot User OAuth Token</p>
                <p className="text-sm text-muted-foreground font-mono">{"*".repeat(12)}</p>
              </div>
            </div>

            {(config?.team_id || config?.channel_id) && (
              <div className="grid grid-cols-2 gap-4">
                {config?.team_id && (
                  <div>
                    <p className="text-sm font-medium">Workspace ID</p>
                    <p className="text-sm text-muted-foreground font-mono">{config.team_id}</p>
                  </div>
                )}
                {config?.channel_id && (
                  <div>
                    <p className="text-sm font-medium">Channel ID</p>
                    <p className="text-sm text-muted-foreground font-mono">{config.channel_id}</p>
                  </div>
                )}
              </div>
            )}

            <div>
              <p className="text-sm font-medium">Session Strategy</p>
              <p className="text-sm text-muted-foreground">
                {config?.session_strategy === "per_thread"
                  ? "Per Thread"
                  : config?.session_strategy === "per_channel"
                    ? "Per Channel"
                    : "Per User"}
              </p>
            </div>

            <div>
              <p className="text-sm font-medium">Reply Mode</p>
              <p className="text-sm text-muted-foreground">
                {config?.reply_mode === "report_progress_only"
                  ? "Report Progress Only"
                  : "All Assistant Messages"}
              </p>
            </div>

            <SlackSetupGuidance
              hasSlackConfig={true}
              isPublished={isPublished}
              webhookVerified={chWebhookVerified}
              firstMessageReceived={chFirstMsg}
              webhookUrl={slackWebhookUrl}
              webhookPath={slackWebhookPath}
              isLocalhost={isLocalhost}
              onCreateSlackApp={handleCreateSlackApp}
              creatingSlackApp={creatingSlackApp}
              onConfigure={() => startEditChannel(channel)}
            />
          </>
        ) : (
          <SlackSetupGuidance
            hasSlackConfig={false}
            isPublished={isPublished}
            webhookVerified={chWebhookVerified}
            firstMessageReceived={chFirstMsg}
            webhookUrl={slackWebhookUrl}
            webhookPath={slackWebhookPath}
            isLocalhost={isLocalhost}
            onCreateSlackApp={handleCreateSlackApp}
            creatingSlackApp={creatingSlackApp}
            onConfigure={() => startEditChannel(channel)}
          />
        )}
      </div>
    );
  };

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/apps"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Apps
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <Rocket className="w-6 h-6" />
            <span className={getEntityNameClassName(app.status)}>{app.name}</span>
            <CopyButton value={app.id} />
            <Badge variant={getEntityStatusBadgeVariant(app.status)}>{app.status}</Badge>
          </h1>
          {app.description && <p className="text-muted-foreground mt-1">{app.description}</p>}
        </div>
        <div className="flex gap-2">
          {isPublished ? (
            <Button
              variant="outline"
              onClick={() => unpublishApp.mutate(app.id)}
              disabled={unpublishApp.isPending || isReadOnly}
            >
              <GlobeLock className="w-4 h-4 mr-2" />
              {unpublishApp.isPending ? "Unpublishing..." : "Unpublish"}
            </Button>
          ) : (
            <Button
              variant="default"
              onClick={() => publishApp.mutate(app.id)}
              disabled={publishApp.isPending || !canPublishApp || isReadOnly}
            >
              <Globe className="w-4 h-4 mr-2" />
              {publishApp.isPending ? "Publishing..." : "Publish"}
            </Button>
          )}
          {!isArchived && (
            <Button
              variant="outline"
              onClick={handleArchive}
              disabled={deleteAppMutation.isPending || isReadOnly}
            >
              {deleteAppMutation.isPending ? "Archiving..." : "Archive"}
            </Button>
          )}
          {isArchived && canDangerousDelete && (
            <Button
              variant="destructive"
              className="text-destructive hover:text-destructive"
              onClick={() => setShowDeleteDialog(true)}
            >
              <Trash2 className="w-4 h-4 mr-2" />
              Delete
            </Button>
          )}
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Channels */}
          {(app.channels ?? []).map((channel: AppChannel) => (
            <Card key={channel.id}>
              <CardHeader className="flex flex-row items-center justify-between">
                <CardTitle className="flex items-center gap-2">
                  <Badge variant="outline">{getChannelTypeDisplayName(channel.channel_type)}</Badge>
                  {!channel.enabled && <Badge variant="secondary">Disabled</Badge>}
                  {(app.channels ?? []).length > 1 && (
                    <span className="text-xs text-muted-foreground font-mono">{channel.id}</span>
                  )}
                </CardTitle>
                <div className="flex gap-1">
                  {editingChannelId !== channel.id && !isReadOnly && (
                    <>
                      <Button variant="outline" size="sm" onClick={() => startEditChannel(channel)}>
                        <Pencil className="w-3 h-3 mr-1" />
                        {channel.channel_type === "slack" &&
                        (channel.channel_config as SlackChannelConfig)?.signing_secret
                          ? "Edit"
                          : "Configure"}
                      </Button>
                      {(app.channels ?? []).length > 1 && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => deleteChannelMutation.mutate(channel.id)}
                          disabled={deleteChannelMutation.isPending}
                        >
                          <Trash2 className="w-3 h-3" />
                        </Button>
                      )}
                    </>
                  )}
                </div>
              </CardHeader>
              <CardContent>
                {editingChannelId === channel.id
                  ? renderChannelForm(updateChannelMutation.isPending, channel.id)
                  : renderChannelDisplay(channel)}
              </CardContent>
            </Card>
          ))}

          {/* Add Channel button */}
          {!isReadOnly && !showAddChannel && (
            <Button variant="outline" className="w-full" onClick={startAddChannel}>
              <Plus className="w-4 h-4 mr-2" />
              Add Channel
            </Button>
          )}

          {appBudgetsEnabled && <AppBudgetsCard app={app} readOnly={isReadOnly} />}

          {/* Add Channel form */}
          {showAddChannel && (
            <Card>
              <CardHeader>
                <CardTitle>Add Channel</CardTitle>
              </CardHeader>
              <CardContent>{renderChannelForm(addChannelMutation.isPending, "new")}</CardContent>
            </Card>
          )}

          {/* Webhook URL Card - shown when published and configured */}
          {isPublished && hasSlackConfig && (
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Globe className="w-5 h-5" />
                  Event Subscriptions
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                {isLocalhost ? (
                  <>
                    <p className="text-sm text-muted-foreground">
                      Slack can&apos;t reach <code className="text-xs">localhost</code>. Use{" "}
                      <a
                        href="https://ngrok.com"
                        target="_blank"
                        rel="noopener noreferrer"
                        className="underline"
                      >
                        ngrok
                      </a>{" "}
                      to expose your local server:
                    </p>
                    <div className="bg-muted p-3 space-y-1">
                      <p className="text-xs font-medium text-muted-foreground">1. Start ngrok:</p>
                      <code className="text-xs block">
                        ngrok http{" "}
                        {typeof window !== "undefined" ? window.location.port || "9300" : "9300"}
                      </code>
                      <p className="text-xs font-medium text-muted-foreground mt-2">
                        2. Copy your Request URL:
                      </p>
                      <code className="text-xs block">
                        https://&lt;your-id&gt;.ngrok-free.app{slackWebhookPath}
                      </code>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="text-sm text-muted-foreground">
                      Paste this URL in your Slack app &rarr; <strong>Event Subscriptions</strong>{" "}
                      &rarr; <strong>Request URL</strong>.
                    </p>
                    <div className="flex items-center gap-2 bg-muted p-3">
                      <Globe className="w-4 h-4 shrink-0 text-muted-foreground" />
                      <code className="text-sm flex-1 truncate">{slackWebhookUrl}</code>
                      <CopyButton value={slackWebhookUrl} />
                    </div>
                  </>
                )}
              </CardContent>
            </Card>
          )}
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Configuration</CardTitle>
              {!editingBasic && !isReadOnly && (
                <Button variant="outline" size="sm" onClick={startEditBasic}>
                  <Pencil className="w-3 h-3 mr-1" />
                  Edit
                </Button>
              )}
            </CardHeader>
            <CardContent>
              {editingBasic ? (
                <div className="space-y-4">
                  <div>
                    <Label htmlFor="edit_name">Name</Label>
                    <Input
                      id="edit_name"
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      required
                    />
                  </div>

                  <div>
                    <Label htmlFor="edit_description">Description</Label>
                    <Input
                      id="edit_description"
                      value={editDescription}
                      onChange={(e) => setEditDescription(e.target.value)}
                    />
                  </div>

                  <div>
                    <Label>Harness</Label>
                    <HarnessSelect value={editHarnessId} onValueChange={setEditHarnessId} />
                  </div>

                  <div>
                    <Label>Agent</Label>
                    <AgentSelect value={editAgentId} onValueChange={setEditAgentId} />
                  </div>

                  <div>
                    <Label>Agent identity</Label>
                    <AgentIdentitySelect
                      value={editAgentIdentityId}
                      onValueChange={setEditAgentIdentityId}
                    />
                  </div>

                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      onClick={saveBasic}
                      disabled={updateApp.isPending || !editName || !editHarnessId}
                    >
                      <Check className="w-3 h-3 mr-1" />
                      {updateApp.isPending ? "Saving..." : "Save"}
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => setEditingBasic(false)}>
                      <X className="w-3 h-3 mr-1" />
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="space-y-4">
                  <div>
                    <p className="text-sm font-medium">Harness</p>
                    <p
                      className={`text-sm ${getEntityReferenceClassName(harness?.status ?? "deleted")}`}
                    >
                      {getEntityReferenceLabel({
                        kind: "Harness",
                        name: getDisplayName(harness),
                        status: harness?.status ?? "deleted",
                      })}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Agent</p>
                    <p
                      className={`text-sm ${getEntityReferenceClassName(
                        app.agent_id ? (agent?.status ?? "deleted") : undefined,
                      )}`}
                    >
                      {app.agent_id
                        ? getEntityReferenceLabel({
                            kind: "Agent",
                            name: getDisplayName(agent),
                            status: agent?.status ?? "deleted",
                          })
                        : "None assigned"}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Channels</p>
                    <div className="flex gap-1 flex-wrap">
                      {(app.channels ?? []).map((ch: AppChannel) => (
                        <Badge key={ch.id} variant="outline">
                          {getChannelTypeDisplayName(ch.channel_type)}
                        </Badge>
                      ))}
                      {(app.channels ?? []).length === 0 && (
                        <span className="text-sm text-muted-foreground">None configured</span>
                      )}
                    </div>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Created</p>
                    <p className="text-sm text-muted-foreground">
                      {new Date(app.created_at).toLocaleString()}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Updated</p>
                    <p className="text-sm text-muted-foreground">
                      {new Date(app.updated_at).toLocaleString()}
                    </p>
                  </div>

                  {app.published_at && (
                    <div>
                      <p className="text-sm font-medium">Published</p>
                      <p className="text-sm text-muted-foreground">
                        {new Date(app.published_at).toLocaleString()}
                      </p>
                    </div>
                  )}
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>

      {/* Delete confirmation dialog */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete App</DialogTitle>
            <DialogDescription>
              Permanently delete the archived app &quot;{app.name}&quot;? Existing references will
              render as deleted tombstones.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDelete}
              disabled={destroyAppMutation.isPending}
            >
              {destroyAppMutation.isPending ? "Deleting..." : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
