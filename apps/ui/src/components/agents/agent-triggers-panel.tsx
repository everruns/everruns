"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { usePolicies } from "@/hooks/use-policies";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { usePageFormPrefill } from "@/components/chat-mode/use-page-form-prefill";
import { useState, type ReactNode } from "react";
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
  scheduleTriggerDraftSchema,
  type TriggerFormState,
} from "@/components/agents/trigger-form";
import {
  getScheduleTriggerConfig,
  getWebhookTriggerConfig,
  TriggerSetupGuidance,
} from "@/components/agents/integrations/trigger-setup-guidance";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { EmptyState } from "@/components/layout";
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
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const { data: runs = [], isLoading } = useAgentTriggerRuns(agentId, triggerId);
  if (isLoading)
    return <p className="text-xs text-muted-foreground">{ui("Loading recent outcomes…")}</p>;
  if (runs.length === 0)
    return <p className="text-xs text-muted-foreground">{ui("No runs yet.")}</p>;
  return (
    <div className="flex flex-wrap gap-2">
      {runs.slice(0, 5).map((run) => (
        <span key={run.id} className="inline-flex items-center gap-1 text-xs text-muted-foreground">
          <Badge variant={run.status === "completed" ? "default" : "outline"}>{run.status}</Badge>
          {formatDistanceToNow(new Date(run.scheduled_at), { addSuffix: true }, displayLocale)}
        </span>
      ))}
    </div>
  );
}

export function AgentTriggersPanel({
  agentId,
  children,
}: {
  agentId: string;
  children?: ReactNode;
}) {
  const ui = useUiTranslations();
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
  const policy = usePolicies("agents");
  const canManage = !policy.isLoading && !policy.error && policy.can("agent.manage");
  const savedTrigger = triggers.find((trigger) => trigger.id === editing?.id);
  const savedConfig = savedTrigger ? getScheduleTriggerConfig(savedTrigger) : undefined;
  const saved = {
    cron_expression: savedConfig?.cron_expression ?? EMPTY_TRIGGER_FORM.cron_expression,
    timezone: savedConfig?.timezone ?? "UTC",
    session_mode: savedConfig?.session_mode ?? "shared_session",
    message: savedConfig?.message ?? "",
    enabled: savedTrigger?.enabled ?? true,
  };
  const values = {
    ...form,
    timezone: form.timezone ?? "UTC",
    session_mode: form.session_mode ?? "shared_session",
  };
  usePageFormPrefill({
    identity: editing?.id ?? "none",
    open: dialogOpen,
    values,
    saved,
    apply: setForm,
  });
  const chatDraft = usePageFormDraft({
    schema: scheduleTriggerDraftSchema,
    entityType: "agent_trigger",
    entityId: editing?.id,
    formId: "schedule",
    label: "Edit trigger",
    priority: 30,
    enabled: dialogOpen && !!savedTrigger,
    baseline: savedTrigger?.updated_at ?? "new",
    values,
    dirty: JSON.stringify(values) !== JSON.stringify(saved),
    writable: canManage && !updateTrigger.isPending,
    apply: setForm,
  });

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
    if (!editing || !canManage || chatDraft.blocked || updateTrigger.isPending) return;
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
    <section id="triggers" className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h3 className="text-lg font-semibold tracking-tight">{ui("Triggers")}</h3>
          <p className="text-sm text-muted-foreground">
            {ui("When this agent wakes up on its own")}
          </p>
        </div>
        <LinkButton href={`/agents/${agentId}/triggers/new`} size="sm">
          <Plus className="size-4" /> {ui("Add trigger")}
        </LinkButton>
      </div>
      {children}
      <div className="space-y-3">
        {isLoading ? (
          <p className="text-sm text-muted-foreground">{ui("Loading triggers…")}</p>
        ) : triggers.length === 0 ? (
          <EmptyState
            icon={<Clock3 />}
            title={ui("No triggers yet")}
            description={ui("Add a schedule or event to run this agent proactively.")}
            className="border-solid bg-card py-8"
          />
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
              <div key={trigger.id} className="space-y-3 border bg-card p-4">
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
                        <span className="font-medium">{ui("GitHub pull requests")}</span>
                      ) : isMcpEvent ? (
                        <span className="font-medium">
                          {ui("MCP event")} {mcpEventConfig.event} {ui("on")}{" "}
                          {mcpEventConfig.server}
                        </span>
                      ) : isSchedule ? (
                        <CronLabel
                          expr={getScheduleTriggerConfig(trigger).cron_expression}
                          tz={getScheduleTriggerConfig(trigger).timezone}
                        />
                      ) : (
                        <span className="font-medium">{ui("Webhook")}</span>
                      )}
                      <Badge variant={trigger.enabled ? "default" : "outline"}>
                        {trigger.enabled ? ui("Enabled") : ui("Disabled")}
                      </Badge>
                    </div>
                    <p className="text-sm text-muted-foreground">{config.message}</p>
                    {isGithub && (
                      <p className="text-xs text-muted-foreground">
                        {(githubConfig.events ?? []).join(", ")} ·{" "}
                        {githubConfig.repositories?.length
                          ? githubConfig.repositories.join(", ")
                          : ui("all repositories")}
                      </p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      {config.session_mode === "session_per_invocation"
                        ? ui("New session per run")
                        : config.session_mode === "per_thread"
                          ? ui("One session per pull request")
                          : ui("One session for everything")}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Switch
                      disabled={!canManage || updateTrigger.isPending}
                      checked={trigger.enabled}
                      aria-label={trigger.enabled ? ui("Disable trigger") : ui("Enable trigger")}
                      onCheckedChange={(enabled) =>
                        updateTrigger.mutate({ triggerId: trigger.id, request: { enabled } })
                      }
                    />
                    {isGithub && (
                      <LinkButton
                        href={`/agents/${agentId}/triggers/${trigger.id}`}
                        variant="ghost"
                        size="icon-sm"
                        aria-label={ui("Open trigger editor")}
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
                          disabled={!canManage || !trigger.enabled || runTrigger.isPending}
                        >
                          <Play className="size-4" /> {ui("Run now")}
                        </Button>
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          disabled={!canManage}
                          onClick={() => openEdit(trigger)}
                        >
                          <Pencil className="size-4" />
                          <span className="sr-only">{ui("Quick edit trigger")}</span>
                        </Button>
                        <LinkButton
                          href={`/agents/${agentId}/triggers/${trigger.id}`}
                          variant="ghost"
                          size="icon-sm"
                          aria-label={ui("Open trigger editor")}
                        >
                          <ExternalLink className="size-4" />
                        </LinkButton>
                      </>
                    )}
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      disabled={!canManage || deleteTrigger.isPending}
                      onClick={() => deleteTrigger.mutate(trigger.id)}
                    >
                      <Trash2 className="size-4" />
                      <span className="sr-only">{ui("Delete trigger")}</span>
                    </Button>
                  </div>
                </div>
                {isSchedule && <TriggerRuns agentId={agentId} triggerId={trigger.id} />}
                {!isGithub && !isMcpEvent && (
                  <div className="border-t pt-4">
                    <p className="mb-3 text-xs font-medium text-muted-foreground">{ui("Set up")}</p>
                    <TriggerSetupGuidance trigger={trigger} />
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{ui("Edit trigger")}</DialogTitle>
            <DialogDescription>
              {ui("Configure when the agent wakes and what message starts the run.")}
            </DialogDescription>
          </DialogHeader>
          <TriggerFormFields value={form} onChange={setForm} />
          {chatDraft.notice}
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)}>
              {ui("Cancel")}
            </Button>
            {chatDraft.blocked && (
              <Button variant="outline" onClick={() => setForm(saved)}>
                {ui("Discard")}
              </Button>
            )}
            <Button
              onClick={save}
              disabled={!canManage || updateTrigger.isPending || chatDraft.blocked}
            >
              {ui("Save changes")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
