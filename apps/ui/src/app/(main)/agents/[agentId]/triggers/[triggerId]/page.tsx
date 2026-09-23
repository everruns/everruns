"use client";

import { use, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { CalendarClock, Check, Pencil, Play, Trash2 } from "lucide-react";
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

/// Full-page trigger editor, matching the channel editors rather than the
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
  const [error, setError] = useState<string | null>(null);

  const trigger = isNew ? undefined : triggers.find((candidate) => candidate.id === triggerId);
  const returnHref = `/agents/${agentId}?tab=integrations`;

  useEffect(() => {
    if (!trigger) return;
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
    if (!isTriggerFormValid(form)) {
      setError("Enter a valid schedule and a non-empty message.");
      return;
    }
    setError(null);
    try {
      if (trigger) {
        await updateTrigger.mutateAsync({ triggerId: trigger.id, request: form });
      } else {
        await createTrigger.mutateAsync(form);
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
        icon={<CalendarClock />}
        title={title}
        badges={
          <>
            {!isNew && (
              <Badge variant="accent">
                <Pencil className="size-3" />
                Editing
              </Badge>
            )}
            <Badge variant={form.enabled ? "default" : "secondary"}>
              {form.enabled ? "enabled" : "disabled"}
            </Badge>
          </>
        }
        description={
          // Read-only surfaces always render the cadence, never the raw cron.
          <>
            <CronLabel expr={form.cron_expression} tz={form.timezone} /> · {agentName}
          </>
        }
        actions={
          <>
            <Button
              type="submit"
              form="trigger-edit-form"
              disabled={
                !isTriggerFormValid(form) || createTrigger.isPending || updateTrigger.isPending
              }
            >
              <Check className="size-4" />
              {createTrigger.isPending || updateTrigger.isPending ? "Saving..." : "Save"}
            </Button>
            {trigger && (
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
                <CardTitle>Schedule</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <TriggerFormFields value={form} onChange={setForm} />
                {error && <p className="text-sm text-destructive">{error}</p>}
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
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
