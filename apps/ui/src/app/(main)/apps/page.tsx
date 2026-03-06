"use client";

import { useState } from "react";
import {
  useApps,
  useCreateApp,
  useDeleteApp,
  usePublishApp,
  useUnpublishApp,
  useUpdateApp,
} from "@/hooks/use-apps";
import { useAgents } from "@/hooks";
import { useHarnesses } from "@/hooks/use-harnesses";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Plus, Rocket, Trash2, Globe, GlobeLock, Copy } from "lucide-react";
import type { App, CreateAppRequest, SessionStrategy, SlackChannelConfig } from "@/lib/api/types";

export default function AppsPage() {
  const { data: apps, isLoading, error } = useApps();
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [editingApp, setEditingApp] = useState<App | null>(null);
  const [deletingApp, setDeletingApp] = useState<App | null>(null);

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Error loading apps: {error.message}</div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Apps</h1>
        <Button variant="accent" onClick={() => setShowCreateDialog(true)}>
          <Plus className="w-4 h-4 mr-2" />
          New App
        </Button>
      </div>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[...Array(3)].map((_, i) => (
            <Card key={i}>
              <CardHeader>
                <Skeleton className="h-6 w-3/4" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-4 w-full mb-2" />
                <Skeleton className="h-4 w-2/3" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : apps && apps.length > 0 ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {apps.map((app) => (
            <AppCard
              key={app.id}
              app={app}
              onEdit={() => setEditingApp(app)}
              onDelete={() => setDeletingApp(app)}
            />
          ))}
        </div>
      ) : (
        <EmptyState onCreateClick={() => setShowCreateDialog(true)} />
      )}

      <AppFormDialog open={showCreateDialog} onOpenChange={setShowCreateDialog} mode="create" />

      {editingApp && (
        <AppFormDialog
          open={true}
          onOpenChange={(open) => !open && setEditingApp(null)}
          mode="edit"
          app={editingApp}
        />
      )}

      {deletingApp && (
        <DeleteConfirmDialog
          app={deletingApp}
          open={true}
          onOpenChange={(open) => !open && setDeletingApp(null)}
        />
      )}
    </div>
  );
}

function AppCard({
  app,
  onEdit,
  onDelete,
}: {
  app: App;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();
  const isPublished = app.status === "published";
  const webhookUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}/api/v1/apps/${app.id}/slack/events`
      : `/api/v1/apps/${app.id}/slack/events`;

  return (
    <Card className="cursor-pointer hover:border-accent/50 transition-colors" onClick={onEdit}>
      <CardHeader className="pb-2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Rocket className="w-5 h-5 text-muted-foreground" />
            <h3 className="font-semibold text-lg">{app.name}</h3>
          </div>
          <Badge variant={isPublished ? "default" : "secondary"}>{app.status}</Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {app.description && <p className="text-sm text-muted-foreground">{app.description}</p>}
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Badge variant="outline" className="text-xs">
            {app.channel_type}
          </Badge>
          <span>Agent: {app.agent_id}</span>
        </div>

        {isPublished && (
          <div className="flex items-center gap-1 text-xs text-muted-foreground bg-muted p-2 rounded">
            <Globe className="w-3 h-3 shrink-0" />
            <span className="truncate font-mono">{webhookUrl}</span>
            <button
              className="shrink-0 ml-1 hover:text-foreground"
              onClick={(e) => {
                e.stopPropagation();
                navigator.clipboard.writeText(webhookUrl);
              }}
            >
              <Copy className="w-3 h-3" />
            </button>
          </div>
        )}

        {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
        <div className="flex gap-2 pt-2" role="group" onClick={(e) => e.stopPropagation()}>
          {isPublished ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => unpublishApp.mutate(app.id)}
              disabled={unpublishApp.isPending}
            >
              <GlobeLock className="w-3 h-3 mr-1" />
              Unpublish
            </Button>
          ) : (
            <Button
              variant="default"
              size="sm"
              onClick={() => publishApp.mutate(app.id)}
              disabled={publishApp.isPending}
            >
              <Globe className="w-3 h-3 mr-1" />
              Publish
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive hover:text-destructive"
            onClick={onDelete}
          >
            <Trash2 className="w-3 h-3" />
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function EmptyState({ onCreateClick }: { onCreateClick: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <Rocket className="w-12 h-12 text-muted-foreground mb-4" />
      <h3 className="text-lg font-semibold mb-2">No Apps Yet</h3>
      <p className="text-muted-foreground mb-4 max-w-md">
        Apps deploy your agents to channels like Slack. Create an app to connect an agent to a Slack
        workspace.
      </p>
      <Button variant="accent" onClick={onCreateClick}>
        <Plus className="w-4 h-4 mr-2" />
        Create Your First App
      </Button>
    </div>
  );
}

function AppFormDialog({
  open,
  onOpenChange,
  mode,
  app,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mode: "create" | "edit";
  app?: App;
}) {
  const { data: agents } = useAgents();
  const { data: harnesses } = useHarnesses();
  const createApp = useCreateApp();
  const updateAppMutation = useUpdateApp();

  const existingConfig = app?.channel_config as SlackChannelConfig | undefined;

  const [name, setName] = useState(app?.name ?? "");
  const [description, setDescription] = useState(app?.description ?? "");
  const [agentId, setAgentId] = useState(app?.agent_id ?? "");
  const [harnessId, setHarnessId] = useState(app?.harness_id ?? "");
  const [signingSecret, setSigningSecret] = useState(existingConfig?.signing_secret ?? "");
  const [botToken, setBotToken] = useState(existingConfig?.bot_token ?? "");
  const [channelId, setChannelId] = useState(existingConfig?.channel_id ?? "");
  const [teamId, setTeamId] = useState(existingConfig?.team_id ?? "");
  const [sessionStrategy, setSessionStrategy] = useState<SessionStrategy>(
    existingConfig?.session_strategy ?? "per_thread",
  );

  const isPending = createApp.isPending || updateAppMutation.isPending;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    const channelConfig: SlackChannelConfig = {
      signing_secret: signingSecret,
      bot_token: botToken,
      session_strategy: sessionStrategy,
      ...(channelId ? { channel_id: channelId } : {}),
      ...(teamId ? { team_id: teamId } : {}),
    };

    if (mode === "create") {
      const req: CreateAppRequest = {
        name,
        description: description || undefined,
        agent_id: agentId,
        harness_id: harnessId,
        channel_type: "slack",
        channel_config: channelConfig,
      };
      await createApp.mutateAsync(req);
    } else if (app) {
      await updateAppMutation.mutateAsync({
        appId: app.id,
        data: {
          name,
          description: description || undefined,
          agent_id: agentId,
          harness_id: harnessId,
          channel_config: channelConfig,
        },
      });
    }

    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>{mode === "create" ? "Create Slack App" : "Edit Slack App"}</DialogTitle>
            <DialogDescription>
              Connect an agent to a Slack workspace. You&apos;ll need a Slack Bot Token and Signing
              Secret from your Slack app configuration.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4 py-4">
            <div>
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Support Bot"
                required
              />
            </div>

            <div>
              <Label htmlFor="description">Description</Label>
              <Input
                id="description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Customer support bot for #help channel"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor="harness">Harness</Label>
                <Select value={harnessId} onValueChange={setHarnessId} required>
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
                <Label htmlFor="agent">Agent</Label>
                <Select value={agentId} onValueChange={setAgentId} required>
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
            </div>

            <hr className="my-2" />
            <h4 className="font-medium text-sm">Slack Configuration</h4>

            <div>
              <Label htmlFor="signing_secret">Signing Secret</Label>
              <Input
                id="signing_secret"
                type="password"
                value={signingSecret}
                onChange={(e) => setSigningSecret(e.target.value)}
                placeholder="Your Slack app signing secret"
                required
              />
              <p className="text-xs text-muted-foreground mt-1">
                Found in Slack App &rarr; Basic Information &rarr; App Credentials
              </p>
            </div>

            <div>
              <Label htmlFor="bot_token">Bot Token</Label>
              <Input
                id="bot_token"
                type="password"
                value={botToken}
                onChange={(e) => setBotToken(e.target.value)}
                placeholder="xoxb-..."
                required
              />
              <p className="text-xs text-muted-foreground mt-1">
                Found in Slack App &rarr; OAuth &amp; Permissions
              </p>
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label htmlFor="channel_id">Channel ID (optional)</Label>
                <Input
                  id="channel_id"
                  value={channelId}
                  onChange={(e) => setChannelId(e.target.value)}
                  placeholder="C0123456789"
                />
              </div>

              <div>
                <Label htmlFor="team_id">Team ID (optional)</Label>
                <Input
                  id="team_id"
                  value={teamId}
                  onChange={(e) => setTeamId(e.target.value)}
                  placeholder="T0123456789"
                />
              </div>
            </div>

            <div>
              <Label htmlFor="session_strategy">Session Strategy</Label>
              <Select
                value={sessionStrategy}
                onValueChange={(v) => setSessionStrategy(v as SessionStrategy)}
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
                Controls how Slack messages map to sessions
              </p>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={isPending || !name || !agentId || !harnessId}>
              {isPending ? "Saving..." : mode === "create" ? "Create App" : "Save Changes"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeleteConfirmDialog({
  app,
  open,
  onOpenChange,
}: {
  app: App;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const deleteAppMutation = useDeleteApp();

  const handleDelete = async () => {
    await deleteAppMutation.mutateAsync(app.id);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete App</DialogTitle>
          <DialogDescription>
            Are you sure you want to delete &quot;{app.name}&quot;? This will stop all incoming
            Slack messages from being processed.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
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
  );
}
