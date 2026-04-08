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
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
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
import { SlackSetupGuidance } from "@/components/apps/slack-setup-guidance";
import type {
  AppChannel,
  SessionStrategy,
  SlackChannelConfig,
  SlackReplyMode,
} from "@/lib/api/types";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";

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

  const [editingBasic, setEditingBasic] = useState(false);
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [showAddChannel, setShowAddChannel] = useState(false);

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

  const [creatingSlackApp, setCreatingSlackApp] = useState(false);

  const invalidateApp = () => {
    queryClient.invalidateQueries({ queryKey: queryKeys.apps.detail(appId) });
    queryClient.invalidateQueries({ queryKey: queryKeys.apps.all });
  };

  const updateChannelMutation = useMutation({
    mutationFn: async ({
      channelId,
      config,
    }: {
      channelId: string;
      config: SlackChannelConfig;
    }) => {
      return apiUpdateChannel(appId, channelId, { channel_config: config });
    },
    onSuccess: invalidateApp,
  });

  const addChannelMutation = useMutation({
    mutationFn: async (config: SlackChannelConfig) => {
      return apiAddChannel(appId, { channel_type: "slack", channel_config: config });
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

  const webhookUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${appId}/slack/events`
      : `/api/v1/apps/${appId}/slack/events`;

  const isLocalhost =
    typeof window !== "undefined" &&
    (window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1");
  const webhookPath = `/api/v1/apps/${appId}/slack/events`;

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
    setEditAgentId(app.agent_id);
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
        agent_id: editAgentId,
        agent_identity_id: editAgentIdentityId || null,
        harness_id: editHarnessId,
      },
    });
    setEditingBasic(false);
  };

  const startEditChannel = (channel: AppChannel) => {
    const config = channel.channel_config as SlackChannelConfig;
    setEditSigningSecret(config?.signing_secret ?? "");
    setEditBotToken(config?.bot_token ?? "");
    setEditChannelIdField(config?.channel_id ?? "");
    setEditTeamId(config?.team_id ?? "");
    setEditSessionStrategy(config?.session_strategy ?? "per_thread");
    setEditReplyMode(config?.reply_mode ?? "all_messages");
    setEditingChannelId(channel.id);
  };

  const startAddChannel = () => {
    setEditSigningSecret("");
    setEditBotToken("");
    setEditChannelIdField("");
    setEditTeamId("");
    setEditSessionStrategy("per_thread");
    setEditReplyMode("all_messages");
    setShowAddChannel(true);
  };

  const saveChannel = async () => {
    if (!editingChannelId) return;
    const channelConfig: SlackChannelConfig = {
      signing_secret: editSigningSecret,
      bot_token: editBotToken,
      session_strategy: editSessionStrategy,
      reply_mode: editReplyMode,
      ...(editChannelIdField ? { channel_id: editChannelIdField } : {}),
      ...(editTeamId ? { team_id: editTeamId } : {}),
    };
    await updateChannelMutation.mutateAsync({ channelId: editingChannelId, config: channelConfig });
    setEditingChannelId(null);
  };

  const saveNewChannel = async () => {
    const channelConfig: SlackChannelConfig = {
      signing_secret: editSigningSecret,
      bot_token: editBotToken,
      session_strategy: editSessionStrategy,
      reply_mode: editReplyMode,
      ...(editChannelIdField ? { channel_id: editChannelIdField } : {}),
      ...(editTeamId ? { team_id: editTeamId } : {}),
    };
    await addChannelMutation.mutateAsync(channelConfig);
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
      <div className="container mx-auto p-6">
        <div className="text-red-500">App not found</div>
        <Link href="/apps" className="text-blue-500 hover:underline">
          Back to Apps
        </Link>
      </div>
    );
  }

  const renderChannelForm = (isSaving: boolean, formId: string = "default") => (
    <div className="space-y-4">
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
        <p className="text-xs text-muted-foreground mt-1">
          Found in your Slack app &rarr; Settings &rarr; Basic Information &rarr; App Credentials
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
        <p className="text-xs text-muted-foreground mt-1">
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
          <SelectTrigger>
            <SelectValue>
              {
                {
                  per_thread: "Per Thread (default)",
                  per_channel: "Per Channel",
                  per_user: "Per User",
                }[editSessionStrategy]
              }
            </SelectValue>
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
        <Select value={editReplyMode} onValueChange={(v) => setEditReplyMode(v as SlackReplyMode)}>
          <SelectTrigger>
            <SelectValue>
              {
                {
                  all_messages: "All Assistant Messages",
                  report_progress_only: "Report Progress Only",
                }[editReplyMode]
              }
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all_messages">All Assistant Messages</SelectItem>
            <SelectItem value="report_progress_only">Report Progress Only</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <div className="flex gap-2 pt-2">
        <Button
          size="sm"
          onClick={editingChannelId ? saveChannel : saveNewChannel}
          disabled={isSaving || !editSigningSecret || !editBotToken}
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

  const renderChannelDisplay = (channel: AppChannel) => {
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
              webhookUrl={webhookUrl}
              webhookPath={webhookPath}
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
            webhookUrl={webhookUrl}
            webhookPath={webhookPath}
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
              disabled={publishApp.isPending || !hasSlackConfig || isReadOnly}
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
                  <Badge variant="outline" className="capitalize">
                    {channel.channel_type}
                  </Badge>
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
                        {(channel.channel_config as SlackChannelConfig)?.signing_secret
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

          {/* Add Channel form */}
          {showAddChannel && (
            <Card>
              <CardHeader>
                <CardTitle>Add Slack Channel</CardTitle>
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
                    <div className="bg-muted p-3 rounded-md space-y-1">
                      <p className="text-xs font-medium text-muted-foreground">1. Start ngrok:</p>
                      <code className="text-xs block">
                        ngrok http{" "}
                        {typeof window !== "undefined" ? window.location.port || "9300" : "9300"}
                      </code>
                      <p className="text-xs font-medium text-muted-foreground mt-2">
                        2. Copy your Request URL:
                      </p>
                      <code className="text-xs block">
                        https://&lt;your-id&gt;.ngrok-free.app{webhookPath}
                      </code>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="text-sm text-muted-foreground">
                      Paste this URL in your Slack app &rarr; <strong>Event Subscriptions</strong>{" "}
                      &rarr; <strong>Request URL</strong>.
                    </p>
                    <div className="flex items-center gap-2 bg-muted p-3 rounded-md">
                      <Globe className="w-4 h-4 shrink-0 text-muted-foreground" />
                      <code className="text-sm flex-1 truncate">{webhookUrl}</code>
                      <CopyButton value={webhookUrl} />
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
                      disabled={updateApp.isPending || !editName || !editAgentId || !editHarnessId}
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
                        name: harness?.name,
                        status: harness?.status ?? "deleted",
                      })}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Agent</p>
                    <p
                      className={`text-sm ${getEntityReferenceClassName(agent?.status ?? "deleted")}`}
                    >
                      {getEntityReferenceLabel({
                        kind: "Agent",
                        name: getDisplayName(agent),
                        status: agent?.status ?? "deleted",
                      })}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Channels</p>
                    <div className="flex gap-1 flex-wrap">
                      {(app.channels ?? []).map((ch: AppChannel) => (
                        <Badge key={ch.id} variant="outline" className="capitalize">
                          {ch.channel_type}
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
