"use client";

import { useState } from "react";
import Link from "next/link";
import { Clock3, ExternalLink, Pencil, Play, Plus, Trash2 } from "lucide-react";
import {
  useAgentTriggerRuns,
  useAgentTriggers,
  useDeleteAgentTrigger,
  useRunAgentTrigger,
  useUpdateAgentTrigger,
} from "@/hooks/use-agent-triggers";
import type { AgentTrigger, InvocationSessionMode } from "@/lib/api/types";
import { CronLabel } from "@/components/apps/cron-label";
import {
  EMPTY_TRIGGER_FORM,
  isTriggerFormValid,
  TriggerFormFields,
  type TriggerConfig,
  type TriggerFormState,
} from "@/components/agents/trigger-form";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
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
            Wake this agent on a recurring schedule.
          </p>
        </div>
        <Link href={`/agents/${agentId}/triggers/new`} className={buttonVariants({ size: "sm" })}>
          <Plus className="size-4" /> Add trigger
        </Link>
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
                      <span className="sr-only">Quick edit trigger</span>
                    </Button>
                    <Link
                      href={`/agents/${agentId}/triggers/${trigger.id}`}
                      className={buttonVariants({ variant: "ghost", size: "icon" })}
                      aria-label="Open trigger editor"
                    >
                      <ExternalLink className="size-4" />
                    </Link>
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
