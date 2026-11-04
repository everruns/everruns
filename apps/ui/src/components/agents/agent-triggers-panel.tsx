"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePolicies } from "@/hooks/use-policies";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import { Clock3, Plus } from "lucide-react";
import {
  useAgentTriggers,
  useDeleteAgentTrigger,
  useRunAgentTrigger,
  useUpdateAgentTrigger,
} from "@/hooks/use-agent-triggers";
import type { AgentChannel } from "@/lib/api/types";
import { TriggerList } from "@/components/agents/trigger-list";
import { buttonVariants } from "@/components/ui/button";
import { EmptyState } from "@/components/layout";
import Link from "next/link";

/// The Triggers section of the agent's Channels and triggers tab. Rows match
/// the Channels table above it; settings, setup and run history live on each
/// trigger's own page, so this section has no inline editor.
export function AgentTriggersPanel({
  agentId,
  scheduleChannels = [],
  readOnly = false,
}: {
  agentId: string;
  /** Schedules created as channels before schedules became triggers. */
  scheduleChannels?: AgentChannel[];
  readOnly?: boolean;
}) {
  const ui = useUiTranslations();
  const { data: triggers = [], isLoading } = useAgentTriggers(agentId);
  const updateTrigger = useUpdateAgentTrigger(agentId);
  const deleteTrigger = useDeleteAgentTrigger(agentId);
  const runTrigger = useRunAgentTrigger(agentId);
  const confirm = useConfirmation();
  const policy = usePolicies("agents");
  const canManage = !readOnly && !policy.isLoading && !policy.error && policy.can("agent.manage");
  const empty = triggers.length === 0 && scheduleChannels.length === 0;

  return (
    <section id="triggers" className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0 flex-[1_1_20rem]">
          <h3 className="text-lg font-semibold tracking-tight">{ui("Triggers")}</h3>
          <p className="text-sm text-muted-foreground">
            {ui(
              "When this agent starts on its own: schedules, webhooks, GitHub events and MCP server events.",
            )}
          </p>
        </div>
        {canManage && (
          <Link
            href={`/agents/${agentId}/triggers/new`}
            className={buttonVariants({ variant: "outline" })}
          >
            <Plus className="size-4" />
            {ui("Add trigger")}
          </Link>
        )}
      </div>
      {isLoading ? (
        <p className="text-sm text-muted-foreground">{ui("Loading triggers…")}</p>
      ) : empty ? (
        <EmptyState
          icon={<Clock3 />}
          title={ui("No triggers yet")}
          description={ui("Add a schedule, a webhook or an event to start this agent on its own.")}
          className="border-solid bg-card py-8"
        />
      ) : (
        <TriggerList
          agentId={agentId}
          triggers={triggers}
          scheduleChannels={scheduleChannels}
          onEnabledChange={
            canManage
              ? (trigger, enabled) =>
                  updateTrigger.mutate({ triggerId: trigger.id, request: { enabled } })
              : undefined
          }
          onRunNow={canManage ? (trigger) => runTrigger.mutate(trigger.id) : undefined}
          onDelete={
            canManage
              ? async (trigger) => {
                  if (await confirm(ui("Delete this trigger? It stops starting the agent.")))
                    deleteTrigger.mutate(trigger.id);
                }
              : undefined
          }
          pending={updateTrigger.isPending || deleteTrigger.isPending || runTrigger.isPending}
        />
      )}
    </section>
  );
}
