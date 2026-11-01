"use client";

// Org Home: what needs a person now, then how the agents ran in the last day
// (knowledge/ui/org-home.md). Every section reads an endpoint its owning page
// already uses; Home only merges and ranks. Chat stays the landing route.

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { useChatPageRows } from "@/components/chat-mode/page-view-state";
import { OrganizationIcon } from "@/components/icons/facet-icons";
import { PageContainer, PageMasthead } from "@/components/layout";
import { useAgentActivity } from "@/hooks/use-agent-activity";
import { useHealthIssues } from "@/hooks/use-health-issues";
import { useOrgExposures } from "@/hooks/use-org-exposures";
import { usePolicies } from "@/hooks/use-policies";
import { useModels } from "@/hooks/use-providers";
import { useSandboxFleetStats } from "@/hooks/use-sandbox-fleet";
import { useFilteredSessions } from "@/hooks/use-sessions";
import { useSystemDoctor } from "@/hooks/use-system-doctor";
import { useAgentExamples, useAgents, usePageTitle } from "@/hooks";
import { attentionItems } from "@/lib/agents-home";
import { getMonitoringTimeline } from "@/lib/api/monitoring";
import { getDisplayName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { busiestAgents, countOpenIssues, homeAttentionItems, hourlyTotals } from "@/lib/org-home";
import { EMPTY_SESSION_FILTERS } from "@/lib/session-filters";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";
import { HomeHourlyChart, HomeTimelineCharts } from "./home-activity";
import { HomeAttention } from "./home-attention";
import {
  HomeAgents,
  HomeGetStarted,
  HomeHealth,
  HomeKpis,
  HomeRunning,
  HomeSandboxes,
} from "./home-cards";

const AGENT_ROWS = 6;
const RUNNING_ROWS = 5;
const PAUSED = { ...EMPTY_SESSION_FILTERS, status: ["paused"] };
const RUNNING = { ...EMPTY_SESSION_FILTERS, status: ["running"] };

export function OrgHome() {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  usePageTitle(ui("Home"));
  const { currentOrg } = useOrg();
  const monitoring = useFeatureFlag("monitoring");
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
  const running = useFilteredSessions(RUNNING);
  const doctor = useSystemDoctor({ enabled: canSeeSystem });
  const timeline = useQuery({
    queryKey: ["monitoring-timeline", "organization", { range: "24h" }],
    queryFn: () => getMonitoringTimeline("organization", { range: "24h" }),
    enabled: monitoring,
    refetchInterval: 60_000,
    retry: false,
  });

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

  // Chat beside Home reads the list it shows, so "what should I look at first"
  // is answered from the same ranking.
  useChatPageRows(
    "Needs attention",
    items.map((item) => ({
      source: item.source,
      severity: item.severity,
      title: ui(item.title, item.values),
      subject: item.subject ?? null,
      detail: ui(item.body, item.values),
      link: item.href,
    })),
    { loading: agents.isLoading || health.isLoading },
  );

  const userAgents = (agents.data ?? []).filter((agent) => !agent.is_built_in);
  const activeAgents = userAgents.filter((agent) => !isArchivedStatus(agent.status));
  const loading = agents.isLoading || activity.isLoading;
  const newOrg = !loading && activeAgents.length === 0;
  const liveSandboxes = sandboxes.data?.live_by_provider.reduce((sum, item) => sum + item.count, 0);
  const providers = sandboxes.data?.live_by_provider
    .filter((item) => item.count > 0)
    .map((item) => `${item.key} ${item.count}`)
    .join(" · ");
  const openIssues = countOpenIssues(healthIssues);
  const hourly = useMemo(() => hourlyTotals(activity.data?.agents ?? []), [activity.data]);
  const top = busiestAgents(
    (activity.data?.agents ?? []).filter((item) => names.has(item.agent_id)),
    AGENT_ROWS,
  ).filter((item) => item.runs > 0 || item.running_sessions > 0);

  const activityChart = timeline.data ? (
    <HomeTimelineCharts data={timeline.data} />
  ) : activity.data && hourly.length > 0 && (!monitoring || timeline.isError) ? (
    <HomeHourlyChart hourly={hourly} windowStart={activity.data.window_start} />
  ) : null;

  return (
    <PageContainer className="@container/home">
      <PageMasthead
        icon={<OrganizationIcon />}
        title={currentOrg?.name ?? ui("Home")}
        description={
          newOrg
            ? ui("Your organization is set up. Here is what to do next.")
            : ui("What needs you, and how your agents ran in the last 24 hours.")
        }
      />

      <HomeAttention items={items} loading={loading || health.isLoading} />

      {newOrg ? (
        <div className="grid gap-6 @5xl/home:grid-cols-[minmax(0,1fr)_340px]">
          <HomeGetStarted
            hasModels={(models.data ?? []).some((model) => model.enabled)}
            hasAgent={activeAgents.length > 0}
            hasChannel={channels.some((exposure) => exposure.state === "live")}
          />
          <HomeHealth open={openIssues} doctor={canSeeSystem ? doctor.data : undefined} />
        </div>
      ) : (
        <>
          <HomeKpis
            totals={activity.data?.totals}
            summary={timeline.data?.summary}
            previous={timeline.data?.previous}
            liveSandboxes={liveSandboxes}
            sandboxProviders={providers}
          />
          <div className="grid items-start gap-6 @5xl/home:grid-cols-[minmax(0,1fr)_340px]">
            <div className="flex min-w-0 flex-col gap-6">
              {activityChart}
              <HomeAgents agents={top} names={names} total={activeAgents.length} />
              <HomeRunning
                sessions={(running.data?.data ?? []).slice(0, RUNNING_ROWS)}
                total={running.data?.total ?? 0}
                names={names}
              />
            </div>
            <div className="flex min-w-0 flex-col gap-6">
              <HomeHealth open={openIssues} doctor={canSeeSystem ? doctor.data : undefined} />
              {sandboxes.data && <HomeSandboxes stats={sandboxes.data} />}
            </div>
          </div>
        </>
      )}
    </PageContainer>
  );
}
