"use client";

import { use, useState, useCallback } from "react";
import {
  useApp,
  useUpdateApp,
  useDeleteApp,
  usePublishApp,
  useUnpublishApp,
} from "@/hooks/use-apps";
import { useAgents } from "@/hooks";
import { useHarnesses } from "@/hooks/use-harnesses";
import { getSlackManifest } from "@/lib/api/apps";
import { useRouter } from "next/navigation";
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
import { HarnessSelect } from "@/components/harness/harness-select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ArrowLeft, Globe, GlobeLock, Copy, Trash2, Pencil, Check, X, Rocket } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import { SlackSetupGuidance } from "@/components/apps/slack-setup-guidance";
import type { SessionStrategy, SlackChannelConfig, UpdateAppRequest } from "@/lib/api/types";

export default function AppDetailPage({ params }: { params: Promise<{ appId: string }> }) {
  const { appId } = use(params);
  const router = useRouter();
  const { data: app, isLoading } = useApp(appId);
  const { data: agents } = useAgents();
  const { data: harnesses } = useHarnesses();
  const updateApp = useUpdateApp();
  const deleteAppMutation = useDeleteApp();
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();

  const [editingBasic, setEditingBasic] = useState(false);
  const [editingSlack, setEditingSlack] = useState(false);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);

  // Basic info edit state
  const [editName, setEditName] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editAgentId, setEditAgentId] = useState("");
  const [editHarnessId, setEditHarnessId] = useState("");

  // Slack config edit state
  const [editSigningSecret, setEditSigningSecret] = useState("");
  const [editBotToken, setEditBotToken] = useState("");
  const [editChannelId, setEditChannelId] = useState("");
  const [editTeamId, setEditTeamId] = useState("");
  const [editSessionStrategy, setEditSessionStrategy] = useState<SessionStrategy>("per_thread");

  const [creatingSlackApp, setCreatingSlackApp] = useState(false);

  const isPublished = app?.status === "published";
  const slackConfig = app?.channel_config as SlackChannelConfig | undefined;
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
        harness_id: editHarnessId,
      },
    });
    setEditingBasic(false);
  };

  const startEditSlack = () => {
    setEditSigningSecret(slackConfig?.signing_secret ?? "");
    setEditBotToken(slackConfig?.bot_token ?? "");
    setEditChannelId(slackConfig?.channel_id ?? "");
    setEditTeamId(slackConfig?.team_id ?? "");
    setEditSessionStrategy(slackConfig?.session_strategy ?? "per_thread");
    setEditingSlack(true);
  };

  const saveSlack = async () => {
    if (!app) return;
    const channelConfig: SlackChannelConfig = {
      signing_secret: editSigningSecret,
      bot_token: editBotToken,
      session_strategy: editSessionStrategy,
      ...(editChannelId ? { channel_id: editChannelId } : {}),
      ...(editTeamId ? { team_id: editTeamId } : {}),
    };
    await updateApp.mutateAsync({
      appId: app.id,
      data: { channel_config: channelConfig } as UpdateAppRequest,
    });
    setEditingSlack(false);
  };

  const handleDelete = async () => {
    if (!app) return;
    await deleteAppMutation.mutateAsync(app.id);
    router.push("/apps");
  };

  const agentName = agents?.find((a) => a.id === app?.agent_id)?.name;
  const harnessName = harnesses?.find((h) => h.id === app?.harness_id)?.name;

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
            {app.name}
            <CopyButton value={app.id} />
            <Badge variant={isPublished ? "default" : "secondary"}>{app.status}</Badge>
          </h1>
          {app.description && <p className="text-muted-foreground mt-1">{app.description}</p>}
        </div>
        <div className="flex gap-2">
          {isPublished ? (
            <Button
              variant="outline"
              onClick={() => unpublishApp.mutate(app.id)}
              disabled={unpublishApp.isPending}
            >
              <GlobeLock className="w-4 h-4 mr-2" />
              {unpublishApp.isPending ? "Unpublishing..." : "Unpublish"}
            </Button>
          ) : (
            <Button
              variant="default"
              onClick={() => publishApp.mutate(app.id)}
              disabled={publishApp.isPending || !hasSlackConfig}
            >
              <Globe className="w-4 h-4 mr-2" />
              {publishApp.isPending ? "Publishing..." : "Publish"}
            </Button>
          )}
          <Button
            variant="ghost"
            className="text-destructive hover:text-destructive"
            onClick={() => setShowDeleteDialog(true)}
          >
            <Trash2 className="w-4 h-4" />
          </Button>
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Slack Integration Card */}
          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Slack Integration</CardTitle>
              {!editingSlack && (
                <Button variant="outline" size="sm" onClick={startEditSlack}>
                  <Pencil className="w-3 h-3 mr-1" />
                  {hasSlackConfig ? "Edit" : "Configure"}
                </Button>
              )}
            </CardHeader>
            <CardContent>
              {editingSlack ? (
                <div className="space-y-4">
                  <div>
                    <Label htmlFor="signing_secret">Signing Secret</Label>
                    <Input
                      id="signing_secret"
                      type="password"
                      value={editSigningSecret}
                      onChange={(e) => setEditSigningSecret(e.target.value)}
                      placeholder="Your Slack app's signing secret"
                      required
                    />
                    <p className="text-xs text-muted-foreground mt-1">
                      Found in your Slack app &rarr; Settings &rarr; Basic Information &rarr; App
                      Credentials
                    </p>
                  </div>

                  <div>
                    <Label htmlFor="bot_token">Bot User OAuth Token</Label>
                    <Input
                      id="bot_token"
                      type="password"
                      value={editBotToken}
                      onChange={(e) => setEditBotToken(e.target.value)}
                      placeholder="xoxb-..."
                      required
                    />
                    <p className="text-xs text-muted-foreground mt-1">
                      Found in your Slack app &rarr; OAuth &amp; Permissions &rarr; Bot User OAuth
                      Token
                    </p>
                  </div>

                  <Separator />

                  <div className="grid grid-cols-2 gap-4">
                    <div>
                      <Label htmlFor="team_id">Workspace ID (optional)</Label>
                      <Input
                        id="team_id"
                        value={editTeamId}
                        onChange={(e) => setEditTeamId(e.target.value)}
                        placeholder="T0123456789"
                      />
                      <p className="text-xs text-muted-foreground mt-1">
                        Your Slack workspace (team) ID
                      </p>
                    </div>

                    <div>
                      <Label htmlFor="channel_id">Channel ID (optional)</Label>
                      <Input
                        id="channel_id"
                        value={editChannelId}
                        onChange={(e) => setEditChannelId(e.target.value)}
                        placeholder="C0123456789"
                      />
                      <p className="text-xs text-muted-foreground mt-1">
                        Restrict to a specific channel
                      </p>
                    </div>
                  </div>

                  <div>
                    <Label htmlFor="session_strategy">Session Strategy</Label>
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
                    <p className="text-xs text-muted-foreground mt-1">
                      Controls how Slack messages are grouped into sessions
                    </p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <Button
                      size="sm"
                      onClick={saveSlack}
                      disabled={updateApp.isPending || !editSigningSecret || !editBotToken}
                    >
                      <Check className="w-3 h-3 mr-1" />
                      {updateApp.isPending ? "Saving..." : "Save"}
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => setEditingSlack(false)}>
                      <X className="w-3 h-3 mr-1" />
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : hasSlackConfig ? (
                <div className="space-y-4">
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

                  {(slackConfig?.team_id || slackConfig?.channel_id) && (
                    <div className="grid grid-cols-2 gap-4">
                      {slackConfig?.team_id && (
                        <div>
                          <p className="text-sm font-medium">Workspace ID</p>
                          <p className="text-sm text-muted-foreground font-mono">
                            {slackConfig.team_id}
                          </p>
                        </div>
                      )}
                      {slackConfig?.channel_id && (
                        <div>
                          <p className="text-sm font-medium">Channel ID</p>
                          <p className="text-sm text-muted-foreground font-mono">
                            {slackConfig.channel_id}
                          </p>
                        </div>
                      )}
                    </div>
                  )}

                  <div>
                    <p className="text-sm font-medium">Session Strategy</p>
                    <p className="text-sm text-muted-foreground">
                      {slackConfig?.session_strategy === "per_thread"
                        ? "Per Thread"
                        : slackConfig?.session_strategy === "per_channel"
                          ? "Per Channel"
                          : "Per User"}
                    </p>
                  </div>

                  <SlackSetupGuidance
                    hasSlackConfig={true}
                    isPublished={isPublished}
                    webhookUrl={webhookUrl}
                    webhookPath={webhookPath}
                    isLocalhost={isLocalhost}
                    onCreateSlackApp={handleCreateSlackApp}
                    creatingSlackApp={creatingSlackApp}
                    onConfigure={startEditSlack}
                  />
                </div>
              ) : (
                <SlackSetupGuidance
                  hasSlackConfig={false}
                  isPublished={isPublished}
                  webhookUrl={webhookUrl}
                  webhookPath={webhookPath}
                  isLocalhost={isLocalhost}
                  onCreateSlackApp={handleCreateSlackApp}
                  creatingSlackApp={creatingSlackApp}
                  onConfigure={startEditSlack}
                />
              )}
            </CardContent>
          </Card>

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
                    <p className="text-sm text-muted-foreground">
                      Paste that URL in your Slack app &rarr; <strong>Event Subscriptions</strong>{" "}
                      &rarr; <strong>Request URL</strong>. Then subscribe to bot events:{" "}
                      <code className="text-xs">message.channels</code>,{" "}
                      <code className="text-xs">message.groups</code>,{" "}
                      <code className="text-xs">message.im</code>,{" "}
                      <code className="text-xs">app_mention</code>.
                    </p>
                    <div className="flex items-center gap-2 bg-muted p-2 rounded-md">
                      <code className="text-xs flex-1 truncate text-muted-foreground">
                        {webhookPath}
                      </code>
                      <button
                        className="shrink-0 hover:text-foreground text-muted-foreground"
                        onClick={() => navigator.clipboard.writeText(webhookPath)}
                      >
                        <Copy className="w-4 h-4" />
                      </button>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="text-sm text-muted-foreground">
                      Paste this URL in your Slack app &rarr; <strong>Event Subscriptions</strong>{" "}
                      &rarr; <strong>Request URL</strong>. Then subscribe to bot events:{" "}
                      <code className="text-xs">message.channels</code>,{" "}
                      <code className="text-xs">message.groups</code>,{" "}
                      <code className="text-xs">message.im</code>,{" "}
                      <code className="text-xs">app_mention</code>.
                    </p>
                    <div className="flex items-center gap-2 bg-muted p-3 rounded-md">
                      <Globe className="w-4 h-4 shrink-0 text-muted-foreground" />
                      <code className="text-sm flex-1 truncate">{webhookUrl}</code>
                      <button
                        className="shrink-0 hover:text-foreground text-muted-foreground"
                        onClick={() => navigator.clipboard.writeText(webhookUrl)}
                      >
                        <Copy className="w-4 h-4" />
                      </button>
                    </div>
                  </>
                )}
                <p className="text-xs text-muted-foreground">
                  After saving, invite the bot to a channel (<code>/invite @{app?.name}</code>) and
                  send a message to test.
                </p>
              </CardContent>
            </Card>
          )}
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <CardTitle>Configuration</CardTitle>
              {!editingBasic && (
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
                    <p className="text-sm text-muted-foreground">{harnessName ?? app.harness_id}</p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Agent</p>
                    <p className="text-sm text-muted-foreground">{agentName ?? app.agent_id}</p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Channel</p>
                    <Badge variant="outline">Slack</Badge>
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
              Are you sure you want to delete &quot;{app.name}&quot;? This will stop all incoming
              Slack messages from being processed.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDelete}
              disabled={deleteAppMutation.isPending}
            >
              {deleteAppMutation.isPending ? "Deleting..." : "Delete"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
