"use client";

import { useState } from "react";
import { Clock3, Pencil, Play, Plus, Trash2 } from "lucide-react";
import {
  useAgentTriggerRuns,
  useAgentTriggers,
  useCreateAgentTrigger,
  useDeleteAgentTrigger,
  useRunAgentTrigger,
  useUpdateAgentTrigger,
} from "@/hooks/use-agent-triggers";
import type { AgentTrigger, InvocationSessionMode } from "@/lib/api/types";
import { CronInput, CronLabel, isSupportedCronExpression } from "@/components/apps/cron-label";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { formatDistanceToNow } from "@/lib/formatting";

type TriggerConfig = {
  cron_expression: string;
  timezone?: string;
  session_mode?: InvocationSessionMode;
  message: string;
};

type TriggerForm = TriggerConfig & { enabled: boolean };

const EMPTY_FORM: TriggerForm = {
  cron_expression: "0 0 9 * * * *",
  timezone: "UTC",
  session_mode: "shared_session",
  message: "",
  enabled: true,
};

function configOf(trigger: AgentTrigger): TriggerConfig {
  return trigger.config as TriggerConfig;
}

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
  const createTrigger = useCreateAgentTrigger(agentId);
  const updateTrigger = useUpdateAgentTrigger(agentId);
  const deleteTrigger = useDeleteAgentTrigger(agentId);
  const runTrigger = useRunAgentTrigger(agentId);
  const [editing, setEditing] = useState<AgentTrigger | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [form, setForm] = useState<TriggerForm>(EMPTY_FORM);
  const [error, setError] = useState<string | null>(null);

  const openCreate = () => {
    setEditing(null);
    setForm(EMPTY_FORM);
    setError(null);
    setDialogOpen(true);
  };

  const openEdit = (trigger: AgentTrigger) => {
    const config = configOf(trigger);
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
    if (!isSupportedCronExpression(form.cron_expression) || !form.message.trim()) {
      setError("Enter a valid schedule and a non-empty message.");
      return;
    }
    setError(null);
    try {
      if (editing) {
        await updateTrigger.mutateAsync({ triggerId: editing.id, request: form });
      } else {
        await createTrigger.mutateAsync(form);
      }
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
            Wake this agent on a recurring schedule.
          </p>
        </div>
        <Button onClick={openCreate}>
          <Plus className="size-4" /> Add trigger
        </Button>
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
            const config = configOf(trigger);
            return (
              <div key={trigger.id} className="space-y-3 border p-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="space-y-1">
                    <div className="flex items-center gap-2">
                      <Clock3 className="size-4 text-muted-foreground" />
                      <CronLabel expr={config.cron_expression} tz={config.timezone} />
                      <Badge variant={trigger.enabled ? "default" : "outline"}>
                        {trigger.enabled ? "Enabled" : "Disabled"}
                      </Badge>
                    </div>
                    <p className="text-sm text-muted-foreground">{config.message}</p>
                    <p className="text-xs text-muted-foreground">
                      {config.session_mode === "session_per_invocation"
                        ? "New session per run"
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
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => runTrigger.mutate(trigger.id)}
                      disabled={!trigger.enabled || runTrigger.isPending}
                    >
                      <Play className="size-4" /> Run now
                    </Button>
                    <Button size="icon" variant="ghost" onClick={() => openEdit(trigger)}>
                      <Pencil className="size-4" />
                      <span className="sr-only">Edit trigger</span>
                    </Button>
                    <Button
                      size="icon"
                      variant="ghost"
                      onClick={() => deleteTrigger.mutate(trigger.id)}
                    >
                      <Trash2 className="size-4" />
                      <span className="sr-only">Delete trigger</span>
                    </Button>
                  </div>
                </div>
                <TriggerRuns agentId={agentId} triggerId={trigger.id} />
              </div>
            );
          })
        )}
      </CardContent>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{editing ? "Edit trigger" : "Add trigger"}</DialogTitle>
            <DialogDescription>
              Configure when the agent wakes and what message starts the run.
            </DialogDescription>
          </DialogHeader>
          <CronInput
            value={form.cron_expression}
            timezone={form.timezone ?? "UTC"}
            onChange={(cron_expression) => setForm((value) => ({ ...value, cron_expression }))}
            onTimezoneChange={(timezone) => setForm((value) => ({ ...value, timezone }))}
          />
          <div className="space-y-2">
            <Label htmlFor="trigger-session-mode">Session mode</Label>
            <Select
              value={form.session_mode}
              onValueChange={(session_mode) =>
                setForm((value) => ({
                  ...value,
                  session_mode: session_mode as InvocationSessionMode,
                }))
              }
            >
              <SelectTrigger id="trigger-session-mode" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="shared_session">Shared session</SelectItem>
                <SelectItem value="session_per_invocation">New session per run</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="trigger-message">Message</Label>
            <Textarea
              id="trigger-message"
              value={form.message}
              onChange={(event) => setForm((value) => ({ ...value, message: event.target.value }))}
              placeholder="Run the daily digest"
              rows={4}
            />
          </div>
          <div className="flex items-center gap-2">
            <Switch
              id="trigger-enabled"
              checked={form.enabled}
              onCheckedChange={(enabled) => setForm((value) => ({ ...value, enabled }))}
            />
            <Label htmlFor="trigger-enabled">Enabled</Label>
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={save} disabled={createTrigger.isPending || updateTrigger.isPending}>
              {editing ? "Save changes" : "Create trigger"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
