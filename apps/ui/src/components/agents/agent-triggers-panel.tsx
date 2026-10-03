"use client";

import { useState } from "react";
import {
  Clock3,
  ExternalLink,
  GitPullRequest,
  Pencil,
  Play,
  Plus,
  Radio,
  Trash2,
  Webhook,
} from "lucide-react";
import {
  useAgentTriggerRuns,
  useAgentTriggers,
  useDeleteAgentTrigger,
  useRunAgentTrigger,
  useUpdateAgentTrigger,
} from "@/hooks/use-agent-triggers";
import type { AgentTrigger } from "@/lib/api/types";
import { CronLabel } from "@/components/apps/cron-label";
import {
  EMPTY_TRIGGER_FORM,
  isTriggerFormValid,
  TriggerFormFields,
  type TriggerFormState,
} from "@/components/agents/trigger-form";
import {
  getScheduleTriggerConfig,
  getWebhookTriggerConfig,
  TriggerSetupGuidance,
} from "@/components/agents/integrations/trigger-setup-guidance";
import {
  AgentVersionSelectionBadge,
  versionSelectionOf,
} from "@/components/agents/agent-version-policy-field";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { formatDistanceToNow } from "@/lib/formatting";

function TriggerRuns({ agentId, triggerId }: { agentId: string; triggerId: string }) {
  const { data: runs = [], isLoading } = useAgentTriggerRuns(agentId, triggerId);
  if (isLoading) return <p className="text-xs text-muted-foreground">Loading recent outcomes…</p>;
  if (runs.length === 0) return <p className="text-xs text-muted-foreground">No runs yet.</p>;
  return (
    <div className="flex flex-wrap gap-2">
      {runs.slice(0, 5).map((run) => (
        <span key={run.id} className="inline-flex items-center gap-1 text-xs text-muted-foreground">
          <Badge variant={run.status === "completed" ? "default" : "outline"}>{run.status}</Badge>
          {formatDistanceToNow(new Date(run.scheduled_at), { addSuffix: true })}
        </span>
      ))}
    </div>
  );
}

export function AgentTriggersPanel({ agentId }: { agentId: string }) {
  const { data: triggers = [], isLoading } = useAgentTriggers(agentId);
  const updateTrigger = useUpdateAgentTrigger(agentId);
  const deleteTrigger = useDeleteAgentTrigger(agentId);
  const runTrigger = useRunAgentTrigger(agentId);
  // Quick edit only. Creating a trigger goes to the full-page route
  // (EVE-1009), so there is one create path rather than two that could drift.
  const [editing, setEditing] = useState<AgentTrigger | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [form, setForm] = useState<TriggerFormState>(EMPTY_TRIGGER_FORM);
  const [error, setError] = useState<string | null>(null);

  const openEdit = (trigger: AgentTrigger) => {
    const config = getScheduleTriggerConfig(trigger);
    setEditing(trigger);
    setForm({
      cron_expression: config.cron_expression,
      timezone: config.timezone ?? "UTC",
      session_mode: config.session_mode ?? "shared_session",
      message: config.message,
      enabled: trigger.enabled,
    });
    setError(null);
    setDialogOpen(true);
  };

  const save = async () => {
    if (!editing) return;
    if (!isTriggerFormValid(form)) {
      setError("Enter a valid schedule and a non-empty message.");
      return;
    }
    setError(null);
    try {
      await updateTrigger.mutateAsync({ triggerId: editing.id, request: form });
      setDialogOpen(false);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to save trigger.");
    }
  };

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <div>
          <CardTitle>Triggers</CardTitle>
          <p className="mt-1 text-sm text-muted-foreground">
            Wake this agent on a schedule or on events.
          </p>
        </div>
        <LinkButton href={`/agents/${agentId}/triggers/new`} size="sm">
          <Plus className="size-4" /> Add trigger
        </LinkButton>
      </CardHeader>
      <CardContent className="space-y-3">
        {isLoading ? (
          <p className="text-sm text-muted-foreground">Loading triggers…</p>
        ) : triggers.length === 0 ? (
          <div className="border border-dashed p-8 text-center text-sm text-muted-foreground">
            No triggers yet. Add one to let this agent run proactively.
          </div>
        ) : (
          triggers.map((trigger) => {
            const isGithub = trigger.trigger_type === "github";
            const isSchedule = trigger.trigger_type === "schedule";
            // Created through the API for now; shown read-only here.
            const isMcpEvent = trigger.trigger_type === "mcp_event";
            const mcpEventConfig = trigger.config as unknown as { server?: string; event?: string };
            const githubConfig = trigger.config as unknown as {
              events?: string[];
              repositories?: string[];
              session_mode?: string;
              message: string;
            };
            const config = isGithub
              ? githubConfig
              : isSchedule
                ? getScheduleTriggerConfig(trigger)
                : getWebhookTriggerConfig(trigger);
            return (
              <div key={trigger.id} className="space-y-3 border p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="space-y-1">
                    <div className="flex items-center gap-2">
                      {isGithub ? (
                        <GitPullRequest className="size-4 text-muted-foreground" />
                      ) : isMcpEvent ? (
                        <Radio className="size-4 text-muted-foreground" />
                      ) : isSchedule ? (
                        <Clock3 className="size-4 text-muted-foreground" />
                      ) : (
                        <Webhook className="size-4 text-muted-foreground" />
                      )}
                      {isGithub ? (
                        <span className="font-medium">GitHub pull requests</span>
                      ) : isMcpEvent ? (
                        <span className="font-medium">
                          MCP event {mcpEventConfig.event} on {mcpEventConfig.server}
                        </span>
                      ) : isSchedule ? (
                        <CronLabel
                          expr={getScheduleTriggerConfig(trigger).cron_expression}
                          tz={getScheduleTriggerConfig(trigger).timezone}
                        />
                      ) : (
                        <span className="font-medium">Webhook</span>
                      )}
                      <Badge variant={trigger.enabled ? "default" : "outline"}>
                        {trigger.enabled ? "Enabled" : "Disabled"}
                      </Badge>
                      <AgentVersionSelectionBadge
                        agentId={agentId}
                        selection={versionSelectionOf(trigger)}
                      />
                    </div>
                    <p className="text-sm text-muted-foreground">{config.message}</p>
                    {isGithub && (
                      <p className="text-xs text-muted-foreground">
                        {(githubConfig.events ?? []).join(", ")} ·{" "}
                        {githubConfig.repositories?.length
                          ? githubConfig.repositories.join(", ")
                          : "all repositories"}
                      </p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      {config.session_mode === "session_per_invocation"
                        ? "New session per run"
                        : config.session_mode === "per_thread"
                          ? "One session per pull request"
                          : "Shared session"}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Switch
                      checked={trigger.enabled}
                      aria-label={trigger.enabled ? "Disable trigger" : "Enable trigger"}
                      onCheckedChange={(enabled) =>
                        updateTrigger.mutate({ triggerId: trigger.id, request: { enabled } })
                      }
                    />
                    {isGithub && (
                      <LinkButton
                        href={`/agents/${agentId}/triggers/${trigger.id}`}
                        variant="ghost"
                        size="icon-sm"
                        aria-label="Open trigger editor"
                      >
                        <ExternalLink className="size-4" />
                      </LinkButton>
                    )}
                    {isSchedule && (
                      <>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => runTrigger.mutate(trigger.id)}
                          disabled={!trigger.enabled || runTrigger.isPending}
                        >
                          <Play className="size-4" /> Run now
                        </Button>
                        <Button size="icon-sm" variant="ghost" onClick={() => openEdit(trigger)}>
                          <Pencil className="size-4" />
                          <span className="sr-only">Quick edit trigger</span>
                        </Button>
                        <LinkButton
                          href={`/agents/${agentId}/triggers/${trigger.id}`}
                          variant="ghost"
                          size="icon-sm"
                          aria-label="Open trigger editor"
                        >
                          <ExternalLink className="size-4" />
                        </LinkButton>
                      </>
                    )}
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      onClick={() => deleteTrigger.mutate(trigger.id)}
                    >
                      <Trash2 className="size-4" />
                      <span className="sr-only">Delete trigger</span>
                    </Button>
                  </div>
                </div>
                {isSchedule && <TriggerRuns agentId={agentId} triggerId={trigger.id} />}
                {!isGithub && !isMcpEvent && (
                  <div className="border-t pt-4">
                    <p className="mb-3 text-xs font-medium uppercase text-muted-foreground">
                      Set up
                    </p>
                    <TriggerSetupGuidance trigger={trigger} />
                  </div>
                )}
              </div>
            );
          })
        )}
      </CardContent>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Edit trigger</DialogTitle>
            <DialogDescription>
              Configure when the agent wakes and what message starts the run.
            </DialogDescription>
          </DialogHeader>
          <TriggerFormFields value={form} onChange={setForm} />
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={save} disabled={updateTrigger.isPending}>
              Save changes
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
