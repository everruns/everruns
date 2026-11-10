"use client";

// The ranked "needs attention" list Overview shows, shared with the sidebar so
// the Overview entry can show how many items wait (knowledge/ui/org-home.md).
// Every query here is one Overview already makes; React Query shares the cache.

import { useMemo } from "react";
import { useAgentActivity } from "@/hooks/use-agent-activity";
import { useHealthIssues } from "@/hooks/use-health-issues";
import { useOrgExposures } from "@/hooks/use-org-exposures";
import { usePolicies } from "@/hooks/use-policies";
import { useModels } from "@/hooks/use-providers";
import { useSandboxFleetStats } from "@/hooks/use-sandbox-fleet";
import { useFilteredSessions } from "@/hooks/use-sessions";
import { useSystemDoctor } from "@/hooks/use-system-doctor";
import { useAgentExamples, useAgents } from "@/hooks";
import { attentionItems } from "@/lib/agents-home";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { homeAttentionItems } from "@/lib/org-home";
import { EMPTY_SESSION_FILTERS } from "@/lib/session-filters";
import { useLocale } from "@/providers/locale-provider";

const PAUSED = { ...EMPTY_SESSION_FILTERS, status: ["paused"] };

export function useHomeAttention() {
  const { displayLocale } = useLocale();
  const durable = usePolicies("durable");
  const canSeeSystem = !durable.isLoading && !durable.error && durable.can("durable.view");

  const agents = useAgents({ includeArchived: true });
  const activity = useAgentActivity();
  const examples = useAgentExamples();
  const { exposures } = useOrgExposures();
  const health = useHealthIssues();
  const models = useModels();
  const sandboxes = useSandboxFleetStats({});
  const paused = useFilteredSessions(PAUSED);
  const doctor = useSystemDoctor({ enabled: canSeeSystem });

  const names = useMemo(
    () =>
      new Map((agents.data ?? []).map((agent) => [agent.id, getDisplayName(agent, displayLocale)])),
    [agents.data, displayLocale],
  );
  const activityById = useMemo(
    () => new Map((activity.data?.agents ?? []).map((item) => [item.agent_id, item])),
    [activity.data],
  );
  const channels = useMemo(() => exposures.filter((exposure) => !exposure.isTrigger), [exposures]);
  const healthIssues = useMemo(() => health.data?.data ?? [], [health.data]);

  const items = useMemo(
    () =>
      homeAttentionItems({
        agentItems: attentionItems({
          agents: agents.data ?? [],
          exposures: channels,
          healthIssues,
          models: new Map((models.data ?? []).map((model) => [model.id, model])),
          guidedExamples: new Map(
            (examples.data ?? [])
              .filter((example) => example.setup)
              .map((example) => [example.name, example]),
          ),
          activity: activity.data ? activityById : undefined,
        }),
        healthIssues,
        pausedSessions: paused.data?.data ?? [],
        pausedTotal: paused.data?.total ?? 0,
        activity: activity.data?.agents ?? [],
        agentNames: names,
        sandboxStats: sandboxes.data,
        doctor: canSeeSystem ? doctor.data : undefined,
      }),
    [
      agents.data,
      channels,
      healthIssues,
      models.data,
      examples.data,
      activity.data,
      activityById,
      paused.data,
      names,
      sandboxes.data,
      canSeeSystem,
      doctor.data,
    ],
  );

  return {
    items,
    agents,
    activity,
    health,
    sandboxes,
    names,
    healthIssues,
    models,
    channels,
    doctor: canSeeSystem ? doctor.data : undefined,
  };
}
