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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  ArrowLeft,
  Globe,
  GlobeLock,
  Copy,
  Trash2,
  Pencil,
  Check,
  X,
  Rocket,
  ExternalLink,
  CircleCheck,
  Circle,
} from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
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
              Unpublish
            </Button>
          ) : (
            <Button
              variant="default"
              onClick={() => publishApp.mutate(app.id)}
              disabled={publishApp.isPending || !hasSlackConfig}
            >
              <Globe className="w-4 h-4 mr-2" />
              Publish
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
                        <SelectValue />
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

                  {/* Remaining setup steps after credentials are saved */}
                  {!isPublished && (
                    <>
                      <Separator />
                      <SetupSteps
                        hasSlackConfig={true}
                        isPublished={false}
                        webhookUrl={webhookUrl}
                        onCreateSlackApp={handleCreateSlackApp}
                        creatingSlackApp={creatingSlackApp}
                        onConfigure={startEditSlack}
                      />
                    </>
                  )}
                </div>
              ) : (
                <SetupSteps
                  hasSlackConfig={false}
                  isPublished={isPublished}
                  webhookUrl={webhookUrl}
                  onCreateSlackApp={handleCreateSlackApp}
                  creatingSlackApp={creatingSlackApp}
                  onConfigure={startEditSlack}
                />
              )}
            </CardContent>
          </Card>

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
                    <Select value={editHarnessId} onValueChange={setEditHarnessId}>
                      <SelectTrigger>
                        <SelectValue placeholder="Select harness" />
                      </SelectTrigger>
                      <SelectContent>
                        {harnesses?.map((h) => (
                          <SelectItem key={h.id} value={h.id}>
                            {h.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>

                  <div>
                    <Label>Agent</Label>
                    <Select value={editAgentId} onValueChange={setEditAgentId}>
                      <SelectTrigger>
                        <SelectValue placeholder="Select agent" />
                      </SelectTrigger>
                      <SelectContent>
                        {agents?.map((a) => (
                          <SelectItem key={a.id} value={a.id}>
                            {a.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
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

      {/* Webhook URL Card - shown when published and configured */}
      {isPublished && hasSlackConfig && (
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Globe className="w-5 h-5" />
              Event Subscriptions
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <p className="text-sm text-muted-foreground">
              Paste this URL in your Slack app &rarr; <strong>Event Subscriptions</strong> &rarr;{" "}
              <strong>Request URL</strong>. Then subscribe to bot events:{" "}
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
            <p className="text-xs text-muted-foreground">
              After saving, invite the bot to a channel (<code>/invite @{app?.name}</code>) and
              send a message to test.
            </p>
          </CardContent>
        </Card>
      )}

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

// =========================================================================
// Setup Steps — inline guide for Slack integration setup
// =========================================================================

function StepIcon({ done }: { done: boolean }) {
  return done ? (
    <CircleCheck className="w-5 h-5 text-green-600 shrink-0" />
  ) : (
    <Circle className="w-5 h-5 text-muted-foreground shrink-0" />
  );
}

function SetupSteps({
  hasSlackConfig,
  isPublished,
  webhookUrl,
  onCreateSlackApp,
  creatingSlackApp,
  onConfigure,
}: {
  hasSlackConfig: boolean;
  isPublished: boolean;
  webhookUrl: string;
  onCreateSlackApp: () => void;
  creatingSlackApp: boolean;
  onConfigure: () => void;
}) {
  // Determine the current active step
  const currentStep = !hasSlackConfig ? 1 : !isPublished ? 3 : 4;

  return (
    <div className="space-y-4">
      {!hasSlackConfig && (
        <p className="text-sm text-muted-foreground">
          Follow these steps to connect a Slack bot to this app.
        </p>
      )}

      {/* Step 1: Create Slack App */}
      <div className="flex gap-3">
        <StepIcon done={hasSlackConfig} />
        <div className="flex-1 space-y-1">
          <p className={`text-sm font-medium ${hasSlackConfig ? "text-muted-foreground line-through" : ""}`}>
            1. Create a Slack App
          </p>
          {currentStep === 1 && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                Opens Slack with a pre-filled manifest (bot scopes and settings are already
                configured). Review and click <strong>Create</strong>, then install to your
                workspace.
              </p>
              <Button
                size="sm"
                onClick={onCreateSlackApp}
                disabled={creatingSlackApp}
              >
                <ExternalLink className="w-3 h-3 mr-1" />
                {creatingSlackApp ? "Opening..." : "Create Slack App"}
              </Button>
            </div>
          )}
        </div>
      </div>

      {/* Step 2: Copy credentials */}
      <div className="flex gap-3">
        <StepIcon done={hasSlackConfig} />
        <div className="flex-1 space-y-1">
          <p className={`text-sm font-medium ${hasSlackConfig ? "text-muted-foreground line-through" : ""}`}>
            2. Copy credentials back
          </p>
          {currentStep === 1 && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                After creating the Slack app, copy two values back here:
              </p>
              <ul className="text-xs text-muted-foreground list-disc pl-4 space-y-1">
                <li>
                  <strong>Signing Secret</strong> — Slack app &rarr; Basic Information &rarr; App Credentials
                </li>
                <li>
                  <strong>Bot Token</strong> (<code>xoxb-...</code>) — Slack app &rarr; OAuth &amp; Permissions
                </li>
              </ul>
              <Button size="sm" variant="outline" onClick={onConfigure}>
                <Pencil className="w-3 h-3 mr-1" />
                Configure
              </Button>
            </div>
          )}
        </div>
      </div>

      {/* Step 3: Publish */}
      <div className="flex gap-3">
        <StepIcon done={isPublished} />
        <div className="flex-1 space-y-1">
          <p className={`text-sm font-medium ${isPublished ? "text-muted-foreground line-through" : ""}`}>
            3. Publish the app
          </p>
          {currentStep === 3 && (
            <p className="text-xs text-muted-foreground">
              Click the <strong>Publish</strong> button above to activate the webhook endpoint.
            </p>
          )}
        </div>
      </div>

      {/* Step 4: Event Subscriptions */}
      <div className="flex gap-3">
        <Circle className="w-5 h-5 text-muted-foreground shrink-0" />
        <div className="flex-1 space-y-1">
          <p className="text-sm font-medium">
            4. Configure Event Subscriptions
          </p>
          {currentStep === 4 ? (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                In your Slack app settings, go to <strong>Event Subscriptions</strong>, enable
                events, and paste this Request URL:
              </p>
              <div className="flex items-center gap-2 bg-muted p-2 rounded-md">
                <code className="text-xs flex-1 truncate">{webhookUrl}</code>
                <button
                  className="shrink-0 hover:text-foreground text-muted-foreground"
                  onClick={() => navigator.clipboard.writeText(webhookUrl)}
                >
                  <Copy className="w-3 h-3" />
                </button>
              </div>
              <p className="text-xs text-muted-foreground">
                Then subscribe to bot events: <code>message.channels</code>,{" "}
                <code>message.groups</code>, <code>message.im</code>, <code>app_mention</code>
              </p>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">
              Add the webhook URL to your Slack app&apos;s Event Subscriptions.
            </p>
          )}
        </div>
      </div>

      {/* Step 5: Invite bot */}
      <div className="flex gap-3">
        <Circle className="w-5 h-5 text-muted-foreground shrink-0" />
        <div className="flex-1 space-y-1">
          <p className="text-sm font-medium">
            5. Invite the bot and test
          </p>
          <p className="text-xs text-muted-foreground">
            In Slack, use <code>/invite @botname</code> in a channel, then send a message.
          </p>
        </div>
      </div>
    </div>
  );
}
