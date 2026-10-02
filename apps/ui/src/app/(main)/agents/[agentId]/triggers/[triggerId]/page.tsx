"use client";

import { use, useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { CalendarClock, Check, GitPullRequest, Pencil, Play, Trash2 } from "lucide-react";
import { useAgent } from "@/hooks/use-agents";
import {
  useAgentTriggers,
  useCreateAgentTrigger,
  useDeleteAgentTrigger,
  useRunAgentTrigger,
  useUpdateAgentTrigger,
} from "@/hooks/use-agent-triggers";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ResourceNotFound } from "@/components/resource-not-found";
import { CronLabel } from "@/components/apps/cron-label";
import {
  EMPTY_TRIGGER_FORM,
  isTriggerFormValid,
  TriggerFormFields,
  type TriggerConfig,
  type TriggerFormState,
} from "@/components/agents/trigger-form";
import {
  EMPTY_GITHUB_FORM,
  GitHubTriggerForm,
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
import {
  AgentVersionPolicyField,
  DEFAULT_VERSION_SELECTION,
  isVersionSelectionValid,
  sameVersionSelection,
  useShowVersionSelection,
  versionSelectionOf,
  versionSelectionRequest,
  type AgentVersionSelection,
} from "@/components/agents/agent-version-policy-field";
import { getDisplayName } from "@/lib/entity-lifecycle";

/// Full-page trigger editor, matching the endpoint editors rather than the
/// dialog the inline panel still uses for quick edits (EVE-1009). `new` creates;
/// any other id edits that trigger.
export default function AgentTriggerPage({
  params,
}: {
  params: Promise<{ agentId: string; triggerId: string }>;
}) {
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
  const [githubForm, setGithubForm] = useState<GitHubTriggerFormState>(EMPTY_GITHUB_FORM);
  const [error, setError] = useState<string | null>(null);
  const searchParams = useSearchParams();

  const trigger = isNew ? undefined : triggers.find((candidate) => candidate.id === triggerId);
  const storedVersion = trigger ? versionSelectionOf(trigger) : DEFAULT_VERSION_SELECTION;
  const [versionSelection, setVersionSelection] =
    useState<AgentVersionSelection>(DEFAULT_VERSION_SELECTION);
  const showVersion = useShowVersionSelection(storedVersion);
  // Send the selection only when it changed, so an unrelated save never
  // rewrites a pin (EVE-1139).
  const versionRequest = sameVersionSelection(versionSelection, storedVersion)
    ? {}
    : versionSelectionRequest(versionSelection);
  const returnHref = `/agents/${agentId}?tab=integrations`;
  const isGithub = trigger
    ? trigger.trigger_type === "github"
    : searchParams?.get("type") === "github";

  useEffect(() => {
    if (!trigger) return;
    setVersionSelection(versionSelectionOf(trigger));
    if (trigger.trigger_type === "github") {
      const config = trigger.config as unknown as {
        events?: string[];
        repositories?: string[];
        session_mode?: GitHubSessionMode;
        message: string;
      };
      setGithubForm({
        events: config.events ?? [],
        repositories: config.repositories ?? [],
        session_mode: config.session_mode ?? "per_thread",
        message: config.message,
        enabled: trigger.enabled,
      });
      return;
    }
    const config = trigger.config as TriggerConfig;
    setForm({
      cron_expression: config.cron_expression,
      timezone: config.timezone ?? "UTC",
      session_mode: config.session_mode ?? "shared_session",
      message: config.message,
      enabled: trigger.enabled,
    });
  }, [trigger]);

  const save = async () => {
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
              ...versionRequest,
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
            ...versionRequest,
          });
        }
        router.push(returnHref);
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
          request: { ...form, ...versionRequest },
        });
      } else {
        await createTrigger.mutateAsync({ ...form, ...versionRequest });
      }
      router.push(returnHref);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to save trigger.");
    }
  };

  if (isLoading || agentLoading)
    return <div className="container mx-auto p-6">Loading trigger...</div>;

  if (!agent || (!isNew && !trigger)) {
    return (
      <ResourceNotFound
        title="Trigger not found"
        description="This trigger may have been deleted, moved to another agent, or the URL may be wrong."
        backHref={returnHref}
        backLabel="Back to agent"
        resourceId={triggerId}
      />
    );
  }

  const agentName = getDisplayName(agent);
  const title = isNew ? "New trigger" : "Trigger";
  const enabled = isGithub ? githubForm.enabled : form.enabled;
  const formValid =
    (isGithub ? isGitHubFormValid(githubForm) : isTriggerFormValid(form)) &&
    isVersionSelectionValid(versionSelection);

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentName, href: returnHref },
          { label: title },
        ]}
      />

      <PageMasthead
        icon={isGithub ? <GitPullRequest /> : <CalendarClock />}
        title={title}
        badges={
          <>
            {!isNew && (
              <Badge variant="accent">
                <Pencil className="size-3" />
                Editing
              </Badge>
            )}
            <Badge variant={enabled ? "default" : "secondary"}>
              {enabled ? "enabled" : "disabled"}
            </Badge>
          </>
        }
        description={
          isGithub ? (
            <>Pull request events · {agentName}</>
          ) : (
            // Read-only surfaces always render the cadence, never the raw cron.
            <>
              <CronLabel expr={form.cron_expression} tz={form.timezone} /> · {agentName}
            </>
          )
        }
        actions={
          <>
            <Button
              type="submit"
              form="trigger-edit-form"
              disabled={!formValid || createTrigger.isPending || updateTrigger.isPending}
            >
              <Check className="size-4" />
              {createTrigger.isPending || updateTrigger.isPending ? "Saving..." : "Save"}
            </Button>
            {trigger && !isGithub && (
              <Button
                type="button"
                variant="outline"
                onClick={() => runTrigger.mutate(trigger.id)}
                disabled={!trigger.enabled || runTrigger.isPending}
              >
                <Play className="size-4" />
                Run now
              </Button>
            )}
            <Button type="button" variant="outline" onClick={() => router.push(returnHref)}>
              Discard
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
            <Card>
              <CardHeader>
                <CardTitle>{isGithub ? "GitHub" : "Schedule"}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                {isGithub ? (
                  <GitHubTriggerForm
                    agentId={agentId}
                    value={githubForm}
                    onChange={setGithubForm}
                  />
                ) : (
                  <TriggerFormFields value={form} onChange={setForm} />
                )}
                {error && <p className="text-sm text-destructive">{error}</p>}
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
            {showVersion && (
              <Card className="h-fit">
                <CardHeader>
                  <CardTitle>Agent version</CardTitle>
                </CardHeader>
                <CardContent>
                  <AgentVersionPolicyField
                    agentId={agentId}
                    value={versionSelection}
                    onChange={setVersionSelection}
                    idPrefix="trigger-agent-version"
                  />
                </CardContent>
              </Card>
            )}
            {trigger && (
              <Card className="h-fit border-destructive/50">
                <CardHeader>
                  <CardTitle className="text-destructive">Danger zone</CardTitle>
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
                    Delete trigger
                  </Button>
                </CardContent>
              </Card>
            )}
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href={returnHref}>Back to {agentName}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
