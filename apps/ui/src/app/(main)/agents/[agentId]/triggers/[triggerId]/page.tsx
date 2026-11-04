"use client";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { usePageChatMode } from "@/components/chat-mode/chat-mode-context";

import { z } from "zod";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { usePageFormPrefill } from "@/components/chat-mode/use-page-form-prefill";
import { usePolicies } from "@/hooks/use-policies";

import { use, useEffect, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import {
  CalendarClock,
  Check,
  GitPullRequest,
  Pencil,
  Play,
  Radio,
  Trash2,
  Webhook,
} from "lucide-react";
import Link from "next/link";
import { useAgent } from "@/hooks/use-agents";
import {
  useAgentTriggerRuns,
  useAgentTriggers,
  useCreateAgentTrigger,
  useDeleteAgentTrigger,
  useRunAgentTrigger,
  useUpdateAgentTrigger,
} from "@/hooks/use-agent-triggers";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { TriggerSetupGuidance } from "@/components/agents/integrations/trigger-setup-guidance";
import { lastUsedLabel } from "@/components/agents/channels/channel-summary";
import { triggerSummary } from "@/components/agents/trigger-list";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import { CronLabel } from "@/components/apps/cron-label";
import {
  EMPTY_TRIGGER_FORM,
  isTriggerFormValid,
  TriggerFormFields,
  scheduleTriggerDraftSchema,
  type TriggerConfig,
  type TriggerFormState,
} from "@/components/agents/trigger-form";
import {
  GITHUB_EVENT_OPTIONS,
  GitHubTriggerForm,
  githubEventsLabel,
  githubFormFromParams,
  isGitHubFormValid,
  type GitHubTriggerFormState,
  type GitHubSessionMode,
} from "@/components/agents/github-trigger-form";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import { getDisplayName } from "@/lib/entity-lifecycle";

const githubDraftSchema = z.strictObject({
  message: z.string(),
  events: z.array(z.enum(GITHUB_EVENT_OPTIONS.map((e) => e.value))).max(20),
  repositories: z.array(z.string()).max(100),
  session_mode: z.enum(["per_thread", "shared_session", "session_per_invocation"]),
  enabled: z.boolean(),
});
const scheduleDraftSchema = scheduleTriggerDraftSchema;

/// Full-page trigger page, matching the channel pages (EVE-1009). `new` creates;
/// any other id shows and edits that trigger, and is the trigger's view URL.
///
/// Decision: webhook is a trigger type, not a channel type (EVE-1006). A
/// webhook has no peer waiting for a reply, which is what a trigger is, so it
/// is created here (`?type=webhook`) and New channel no longer offers it.

type TriggerKind = "schedule" | "webhook" | "github" | "mcp_event";

const NEW_TRIGGER_KINDS: { kind: TriggerKind; label: string }[] = [
  { kind: "schedule", label: "Schedule" },
  { kind: "webhook", label: "Webhook" },
  { kind: "github", label: "GitHub" },
];

function newWebhookToken(): string {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function ScheduleRuns({ agentId, triggerId }: { agentId: string; triggerId: string }) {
  const ui = useUiTranslations();
  const { data: runs = [], isLoading } = useAgentTriggerRuns(agentId, triggerId);
  if (isLoading) return <p className="text-sm text-muted-foreground">{ui("Loading runs…")}</p>;
  if (runs.length === 0)
    return <p className="text-sm text-muted-foreground">{ui("No runs yet.")}</p>;
  return (
    <ul className="flex flex-col gap-1.5 text-sm">
      {runs.slice(0, 10).map((run) => (
        <li key={run.id} className="flex items-center justify-between gap-3">
          <Badge variant={run.status === "completed" ? "success" : "outline"}>{run.status}</Badge>
          <span className="text-muted-foreground tabular-nums">
            {lastUsedLabel(run.scheduled_at)}
          </span>
        </li>
      ))}
    </ul>
  );
}
export default function AgentTriggerPage({
  params,
}: {
  params: Promise<{ agentId: string; triggerId: string }>;
}) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { agentId, triggerId } = use(params);
  const isNew = triggerId === "new";
  const router = useRouter();
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const { data: triggers = [], isLoading } = useAgentTriggers(agentId);
  const createTrigger = useCreateAgentTrigger(agentId);
  const updateTrigger = useUpdateAgentTrigger(agentId);
  const deleteTrigger = useDeleteAgentTrigger(agentId);
  const runTrigger = useRunAgentTrigger(agentId);

  const [form, setForm] = useState<TriggerFormState>(EMPTY_TRIGGER_FORM);
  const searchParams = useSearchParams();
  const [githubForm, setGithubForm] = useState<GitHubTriggerFormState>(() =>
    githubFormFromParams(isNew ? searchParams : null),
  );
  const [error, setError] = useState<string | null>(null);

  const trigger = isNew ? undefined : triggers.find((candidate) => candidate.id === triggerId);
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const requestedKind = searchParams?.get("type");
  const kind: TriggerKind = trigger
    ? (trigger.trigger_type as TriggerKind)
    : requestedKind === "github" || requestedKind === "webhook"
      ? requestedKind
      : "schedule";
  const isGithub = kind === "github";
  const isWebhook = kind === "webhook";
  const isMcpEvent = kind === "mcp_event";
  // New webhooks get a generated token; an existing one keeps its token unless
  // a new one is entered.
  const [webhookToken, setWebhookToken] = useState(() => (isNew ? newWebhookToken() : ""));

  const adopted = !!usePageChatMode();
  const seededTrigger = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!trigger || (adopted && seededTrigger.current === trigger.id)) return;
    seededTrigger.current = trigger.id;
    if (trigger.trigger_type === "github") {
      const config = trigger.config as unknown as {
        events?: string[];
        repositories?: string[];
        session_mode?: GitHubSessionMode;
        message: string;
        filter?: GitHubTriggerFormState["filter"];
      };
      setGithubForm({
        events: config.events ?? [],
        repositories: config.repositories ?? [],
        session_mode: config.session_mode ?? "per_thread",
        message: config.message,
        enabled: trigger.enabled,
        filter: config.filter ?? null,
      });
      return;
    }
    const config = trigger.config as TriggerConfig;
    setForm({
      cron_expression: config.cron_expression ?? EMPTY_TRIGGER_FORM.cron_expression,
      timezone: config.timezone ?? "UTC",
      session_mode: config.session_mode ?? "shared_session",
      message: config.message,
      enabled: trigger.enabled,
    });
  }, [trigger, adopted]);

  const { can } = usePolicies("agents");
  const config = trigger?.config as unknown as
    | Partial<Omit<TriggerConfig, "session_mode"> & GitHubTriggerFormState>
    | undefined;
  const savedGithub: z.infer<typeof githubDraftSchema> = {
    message: config?.message ?? "",
    events: config?.events ?? [],
    repositories: config?.repositories ?? [],
    session_mode: config?.session_mode ?? "per_thread",
    enabled: trigger?.enabled ?? true,
  };
  const githubValues = {
    message: githubForm.message,
    events: githubForm.events,
    repositories: githubForm.repositories,
    session_mode: githubForm.session_mode,
    enabled: githubForm.enabled,
  };
  const savedSchedule: z.infer<typeof scheduleDraftSchema> = {
    message: config?.message ?? "",
    cron_expression: config?.cron_expression ?? "",
    timezone: config?.timezone ?? "UTC",
    session_mode:
      config?.session_mode === "session_per_invocation"
        ? "session_per_invocation"
        : "shared_session",
    enabled: trigger?.enabled ?? true,
  };
  const scheduleValues = {
    message: form.message,
    cron_expression: form.cron_expression,
    timezone: form.timezone ?? "UTC",
    session_mode: form.session_mode ?? "shared_session",
    enabled: form.enabled,
  };
  usePageFormPrefill({
    identity: triggerId,
    open: !isNew && isGithub && !!trigger,
    values: githubValues,
    saved: savedGithub,
    apply: (next) =>
      setGithubForm((previous) => ({ ...previous, ...next, filter: config?.filter ?? null })),
  });
  usePageFormPrefill({
    identity: triggerId,
    open: !isNew && !isGithub && !!trigger,
    values: scheduleValues,
    saved: savedSchedule,
    apply: setForm,
  });
  const commonDraft = {
    entityId: isNew ? undefined : triggerId,
    entityType: "agent_trigger",
    label: isNew ? "New trigger" : "Trigger",
    baseline: trigger?.updated_at ?? "new",
    writable:
      !!agent && can("agent.manage") && !createTrigger.isPending && !updateTrigger.isPending,
  };
  const draftEnabled = !!agent && !isLoading && !agentLoading && (isNew || !!trigger);
  const githubDraft = usePageFormDraft({
    ...commonDraft,
    schema: githubDraftSchema,
    formId: "github",
    values: githubValues,
    dirty: isNew || JSON.stringify(githubValues) !== JSON.stringify(savedGithub),
    enabled: draftEnabled && isGithub,
    apply: (next) => setGithubForm((previous) => ({ ...previous, ...next })),
  });
  const scheduleDraft = usePageFormDraft({
    ...commonDraft,
    schema: scheduleDraftSchema,
    formId: "schedule",
    values: scheduleValues,
    dirty: isNew || JSON.stringify(scheduleValues) !== JSON.stringify(savedSchedule),
    enabled: draftEnabled && kind === "schedule",
    apply: setForm,
  });
  const chatDraft = isGithub ? githubDraft : scheduleDraft;
  const save = async () => {
    if (chatDraft.blocked) return;
    if (isGithub) {
      if (!isGitHubFormValid(githubForm)) {
        setError("Select at least one event and enter a non-empty message.");
        return;
      }
      setError(null);
      try {
        if (trigger) {
          await updateTrigger.mutateAsync({
            triggerId: trigger.id,
            request: {
              github_events: githubForm.events,
              repositories: githubForm.repositories,
              session_mode: githubForm.session_mode,
              message: githubForm.message,
              enabled: githubForm.enabled,
              filter: githubForm.filter ?? { conditions: [] },
            },
          });
        } else {
          await createTrigger.mutateAsync({
            trigger_type: "github",
            github_events: githubForm.events,
            repositories: githubForm.repositories,
            session_mode: githubForm.session_mode,
            message: githubForm.message,
            enabled: githubForm.enabled,
            filter: githubForm.filter ?? undefined,
          });
        }
        router.push(returnHref);
      } catch (caught) {
        setError(caught instanceof Error ? caught.message : "Unable to save trigger.");
      }
      return;
    }
    if (isMcpEvent) return;
    if (isWebhook) {
      if (!form.message.trim() || (!trigger && !webhookToken.trim())) {
        setError("Enter a message and a token.");
        return;
      }
      setError(null);
      const token = webhookToken.trim() ? { token: webhookToken.trim() } : {};
      try {
        if (trigger) {
          await updateTrigger.mutateAsync({
            triggerId: trigger.id,
            request: {
              message: form.message,
              session_mode: form.session_mode,
              enabled: form.enabled,
              ...token,
            },
          });
          router.push(returnHref);
        } else {
          const created = await createTrigger.mutateAsync({
            trigger_type: "webhook",
            message: form.message,
            session_mode: form.session_mode,
            enabled: form.enabled,
            ...token,
          });
          // Its page shows the URL to call; the token was shown before saving.
          router.push(`/agents/${agentId}/triggers/${created.id}`);
        }
      } catch (caught) {
        setError(caught instanceof Error ? caught.message : "Unable to save trigger.");
      }
      return;
    }
    if (!isTriggerFormValid(form)) {
      setError("Enter a valid schedule and a non-empty message.");
      return;
    }
    setError(null);
    try {
      if (trigger) {
        await updateTrigger.mutateAsync({
          triggerId: trigger.id,
          request: { ...form },
        });
      } else {
        await createTrigger.mutateAsync({ ...form });
      }
      router.push(returnHref);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to save trigger.");
    }
  };

  if (isLoading || agentLoading)
    return <div className="container mx-auto p-6">{ui("Loading trigger...")}</div>;

  if (!agent || (!isNew && !trigger)) {
    return (
      <ResourceNotFound
        title={ui("Trigger not found")}
        description={ui(
          "This trigger may have been deleted, moved to another agent, or the URL may be wrong.",
        )}
        backHref={returnHref}
        backLabel="Back to agent"
        resourceId={triggerId}
      />
    );
  }

  const agentName = getDisplayName(agent, displayLocale);
  const title = isNew ? "New trigger" : trigger ? triggerSummary(trigger).title : "Trigger";
  const enabled = isGithub ? githubForm.enabled : isMcpEvent ? !!trigger?.enabled : form.enabled;
  const formValid = isGithub
    ? isGitHubFormValid(githubForm)
    : isWebhook
      ? !!form.message.trim() && (!!trigger || !!webhookToken.trim())
      : isMcpEvent
        ? false
        : isTriggerFormValid(form);
  const kindLabel = isGithub
    ? ui("GitHub")
    : isWebhook
      ? ui("Webhook")
      : isMcpEvent
        ? ui("MCP event")
        : ui("Schedule");
  const mcpConfig = (trigger?.config ?? {}) as {
    server?: string;
    event?: string;
    message?: string;
  };

  return (
    <PageContainer>
      {chatDraft.notice}
      <PageBreadcrumb
        items={[
          { label: ui("Agents"), href: "/agents" },
          { label: agentName, href: returnHref },
          { label: title },
        ]}
      />

      <PageMasthead
        icon={
          isGithub ? (
            <GitPullRequest />
          ) : isWebhook ? (
            <Webhook />
          ) : isMcpEvent ? (
            <Radio />
          ) : (
            <CalendarClock />
          )
        }
        title={title}
        badges={
          <>
            {!isNew && (
              <Badge variant="accent">
                <Pencil className="size-3" />
                {ui("Editing")}
              </Badge>
            )}
            <Badge variant={enabled ? "default" : "secondary"}>
              {enabled ? ui("enabled") : ui("disabled")}
            </Badge>
          </>
        }
        description={
          isGithub ? (
            <>
              {githubEventsLabel(githubForm.events)} · {agentName}
            </>
          ) : isWebhook || isMcpEvent ? (
            <>
              {kindLabel} · {agentName}
            </>
          ) : (
            // Read-only surfaces always render the cadence, never the raw cron.
            <>
              <CronLabel expr={form.cron_expression} tz={form.timezone} /> · {agentName}
            </>
          )
        }
        actions={
          <>
            {!isMcpEvent && (
              <Button
                type="submit"
                form="trigger-edit-form"
                disabled={
                  chatDraft.blocked ||
                  !formValid ||
                  createTrigger.isPending ||
                  updateTrigger.isPending
                }
              >
                <Check className="size-4" />
                {createTrigger.isPending || updateTrigger.isPending ? ui("Saving...") : ui("Save")}
              </Button>
            )}
            {trigger && kind === "schedule" && (
              <Button
                type="button"
                variant="outline"
                onClick={() => runTrigger.mutate(trigger.id)}
                disabled={!trigger.enabled || runTrigger.isPending}
              >
                <Play className="size-4" />
                {ui("Run now")}
              </Button>
            )}
            <Button type="button" variant="outline" onClick={() => router.push(returnHref)}>
              {ui("Discard")}
            </Button>
          </>
        }
      />

      <form
        id="trigger-edit-form"
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <PageColumns>
          <PageMain>
            {isNew && (
              <nav aria-label={ui("Trigger type")} className="flex flex-wrap gap-2">
                {NEW_TRIGGER_KINDS.map((option) => (
                  <Link
                    key={option.kind}
                    href={`/agents/${agentId}/triggers/new?type=${option.kind}`}
                    aria-current={option.kind === kind ? "page" : undefined}
                    className={buttonVariants({
                      variant: option.kind === kind ? "default" : "outline",
                      size: "sm",
                    })}
                  >
                    {ui(option.label)}
                  </Link>
                ))}
              </nav>
            )}
            <Card>
              <CardHeader>
                <CardTitle>{kindLabel}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                {isMcpEvent ? (
                  <div className="space-y-2 text-sm">
                    <p>
                      {ui("Starts when {value0} sends {value1}.", {
                        value0: mcpConfig.server ?? "",
                        value1: mcpConfig.event ?? "",
                      })}
                    </p>
                    <p className="text-muted-foreground">{mcpConfig.message}</p>
                    <p className="text-muted-foreground">
                      {ui("MCP event triggers are created and changed through the API and CLI.")}
                    </p>
                  </div>
                ) : isWebhook ? (
                  <>
                    <TriggerFormFields value={form} onChange={setForm} hideSchedule />
                    <div className="space-y-2">
                      <Label htmlFor="webhook-token">{ui("Token")}</Label>
                      <div className="flex gap-2">
                        <Input
                          id="webhook-token"
                          value={webhookToken}
                          onChange={(event) => setWebhookToken(event.target.value)}
                          placeholder={trigger ? ui("Leave empty to keep the current token") : ""}
                          autoComplete="off"
                          className="font-mono"
                        />
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => setWebhookToken(newWebhookToken())}
                        >
                          {ui("Generate")}
                        </Button>
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {ui(
                          "Copy it now: it is not shown again. Callers send it as Authorization: Bearer <token> or X-Everruns-Webhook-Token.",
                        )}
                      </p>
                    </div>
                  </>
                ) : isGithub ? (
                  <GitHubTriggerForm
                    agentId={agentId}
                    value={githubForm}
                    onChange={setGithubForm}
                    // A prefilled new trigger comes back to itself after Connect.
                    returnTo={
                      isNew
                        ? `/agents/${agentId}/triggers/new?${searchParams.toString()}`
                        : undefined
                    }
                  />
                ) : (
                  <TriggerFormFields value={form} onChange={setForm} />
                )}
                {error && <p className="text-sm text-destructive">{error}</p>}
              </CardContent>
            </Card>
            {trigger && (kind === "schedule" || isWebhook) && (
              <Card>
                <CardHeader>
                  <CardTitle>{ui("Set up")}</CardTitle>
                </CardHeader>
                <CardContent>
                  <TriggerSetupGuidance trigger={trigger} />
                </CardContent>
              </Card>
            )}
          </PageMain>

          <PageRail>
            {trigger && kind === "schedule" && (
              <Card className="h-fit">
                <CardHeader>
                  <CardTitle>{ui("Recent runs")}</CardTitle>
                </CardHeader>
                <CardContent>
                  <ScheduleRuns agentId={agentId} triggerId={trigger.id} />
                </CardContent>
              </Card>
            )}
            {trigger && (
              <Card className="h-fit border-destructive/50">
                <CardHeader>
                  <CardTitle className="text-destructive">{ui("Danger zone")}</CardTitle>
                </CardHeader>
                <CardContent>
                  <Button
                    type="button"
                    variant="destructive"
                    onClick={() =>
                      deleteTrigger.mutate(trigger.id, {
                        onSuccess: () => router.push(returnHref),
                      })
                    }
                    disabled={deleteTrigger.isPending}
                  >
                    <Trash2 className="size-4" />
                    {ui("Delete trigger")}
                  </Button>
                </CardContent>
              </Card>
            )}
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href={returnHref}>
          {ui("Back to")} {agentName}
        </BackLink>
      </PageFooter>
    </PageContainer>
  );
}
