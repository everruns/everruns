"use client";

import { useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

// Agents home: the Agents page as the place where a team
// runs its agents rather than a registry of definitions. It answers, for every
// agent, what it is doing now, how it is reached, and how the last day went,
// and it carries the org's channels in a second view that replaces the
// Exposures page. Agent definitions stay on the existing agent page.

import { useCallback, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { ArrowRight, Plus, Radio, Upload } from "lucide-react";
import { AgentImportDialog } from "@/components/agents/agent-import-dialog";
import { ExampleCard } from "@/components/agents";
import { NewAgentLink } from "@/components/agents/new-agent-link";
import { CodingAgentConnectAction } from "@/components/layout/mcp-connect-button";
import { AgentIcon } from "@/components/icons/facet-icons";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import {
  EmptyState,
  PageBreadcrumb,
  PageContainer,
  PageMasthead,
  SectionTabs,
  StatCard,
} from "@/components/layout";
import { useAgentActivity } from "@/hooks/use-agent-activity";
import { useHealthIssues } from "@/hooks/use-health-issues";
import { useOrgExposures } from "@/hooks/use-org-exposures";
import { usePolicies } from "@/hooks/use-policies";
import { useModels } from "@/hooks/use-providers";
import {
  useAgentExamples,
  useAgents,
  useCapabilities,
  useImportAgentExample,
  usePageTitle,
} from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import { importedExampleLanding } from "@/lib/agent-template-setup";
import {
  attentionItems,
  channelShortName,
  channelState,
  fastestFirstReply,
  formatReplyTime,
  matchesAgentTab,
  sortAgents,
  sortChannels,
  type AgentTab,
  type AttentionItem,
  type ChannelState,
} from "@/lib/agents-home";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { formatCompactNumber, pluralize } from "@/lib/formatting";
import type { AgentActivity, AgentActivityTotals, ChannelActivity } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { AgentRow } from "./agent-row";
import { ChannelRow } from "./channel-row";
import { NeedsAttention } from "./needs-attention";

type View = "agents" | "channels";
type ChannelTab = "all" | ChannelState | "public";

const EXAMPLE_PREVIEW_LIMIT = 3;

export function AgentsHome() {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  usePageTitle(ui("Agents"));
  const router = useRouter();
  const searchParams = useSearchParams();
  const view: View = searchParams.get("view") === "channels" ? "channels" : "agents";
  const { currentOrg } = useOrg();
  const { can } = usePolicies("agents");
  const canManage = can("agent.manage");

  const { data: agents, isLoading: agentsLoading } = useAgents({
    includeArchived: true,
  });
  const { data: activity } = useAgentActivity();
  const { data: examples } = useAgentExamples();
  const { exposures, isLoading: exposuresLoading } = useOrgExposures();
  const { data: healthIssues } = useHealthIssues();
  const { data: models } = useModels();
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });

  const [search, setSearch] = useState("");
  const [agentTab, setAgentTab] = useState<AgentTab>("all");
  const [channelTab, setChannelTab] = useState<ChannelTab>("all");
  useChatPageFilters({ search, tab: view, status: view === "channels" ? channelTab : agentTab });

  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importFile, setImportFile] = useState<File | null>(null);

  const setView = useCallback(
    (next: View) => {
      const params = new URLSearchParams(searchParams.toString());
      if (next === "channels") params.set("view", "channels");
      else params.delete("view");
      const query = params.toString();
      router.replace(query ? `/agents?${query}` : "/agents", { scroll: false });
      setSearch("");
    },
    [router, searchParams],
  );

  // Schedules are triggers, not ways in; they show on the agent row instead.
  const channels = useMemo(() => exposures.filter((exposure) => !exposure.isTrigger), [exposures]);

  const activityById = useMemo(
    () => new Map<string, AgentActivity>((activity?.agents ?? []).map((a) => [a.agent_id, a])),
    [activity],
  );
  const channelActivityById = useMemo(
    () =>
      new Map<string, ChannelActivity>((activity?.channels ?? []).map((c) => [c.channel_id, c])),
    [activity],
  );

  const attention = useMemo(
    () =>
      attentionItems({
        agents: agents ?? [],
        exposures: channels,
        healthIssues: healthIssues?.data ?? [],
        models: new Map((models ?? []).map((model) => [model.id, model])),
        guidedExamples: new Map(
          (examples ?? [])
            .filter((example) => example.setup)
            .map((example) => [example.name, example]),
        ),
        // Without activity there is no trigger list, so setup is not judged.
        activity: activity ? activityById : undefined,
      }),
    [agents, channels, healthIssues, models, examples, activity, activityById],
  );
  const attentionByAgent = useMemo(() => {
    const map = new Map<string, AttentionItem[]>();
    for (const item of attention) {
      map.set(item.agentId, [...(map.get(item.agentId) ?? []), item]);
    }
    return map;
  }, [attention]);

  const agentCounts = useMemo(() => {
    const list = agents ?? [];
    const count = (tab: AgentTab) =>
      list.filter((agent) =>
        matchesAgentTab(tab, agent, activityById.get(agent.id), attentionByAgent.has(agent.id)),
      ).length;
    return {
      all: count("all"),
      running: count("running"),
      attention: count("attention"),
      idle: count("idle"),
      archived: count("archived"),
    };
  }, [agents, activityById, attentionByAgent]);

  const visibleAgents = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const filtered = (agents ?? []).filter((agent) => {
      if (
        !matchesAgentTab(
          agentTab,
          agent,
          activityById.get(agent.id),
          attentionByAgent.has(agent.id),
        )
      ) {
        return false;
      }
      if (!needle) return true;
      return [
        agent.display_name,
        getDisplayName(agent, displayLocale),
        agent.name,
        agent.description,
        ...Object.values(agent.localizations ?? {}).flatMap((entry) => [
          entry.display_name,
          entry.description,
        ]),
      ]
        .filter(Boolean)
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
    return sortAgents(filtered, activityById, attentionByAgent);
  }, [agents, agentTab, search, activityById, attentionByAgent, displayLocale]);

  const sessionsByChannel = useMemo(
    () => new Map((activity?.channels ?? []).map((c) => [c.channel_id, c.sessions])),
    [activity],
  );
  const channelCounts = useMemo(() => {
    const count = (predicate: (state: ChannelState, publicLive: boolean) => boolean) =>
      channels.filter((exposure) => predicate(channelState(exposure), exposure.publiclyReachable))
        .length;
    return {
      all: channels.length,
      live: count((state) => state === "live"),
      draft: count((state) => state === "draft"),
      paused: count((state) => state === "paused"),
      public: count((_state, publicLive) => publicLive),
    };
  }, [channels]);
  const visibleChannels = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const filtered = channels.filter((exposure) => {
      const state = channelState(exposure);
      if (channelTab === "public" && !exposure.publiclyReachable) return false;
      if (channelTab !== "all" && channelTab !== "public" && state !== channelTab) return false;
      if (!needle) return true;
      return [
        channelShortName(exposure.channel.channel_type),
        exposure.channel.id,
        exposure.agent ? getDisplayName(exposure.agent, displayLocale) : "",
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
    return sortChannels(filtered, sessionsByChannel);
  }, [channels, channelTab, search, sessionsByChannel, displayLocale]);

  const totals = activity?.totals;
  const orgName = currentOrg?.name ?? "this organization";
  const noAgents = !agentsLoading && (agents ?? []).length === 0;
  // Opening Platform Chat creates a built-in agent during signup. Keep this
  // entry discoverable until the team creates its first agent, without hiding
  // the managed agent from the operational list.
  const onlyBuiltInAgents =
    !agentsLoading && !noAgents && (agents ?? []).every((agent) => agent.is_built_in);

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Agents") }]} />

      <PageMasthead
        icon={<AgentIcon />}
        title={ui("Agents")}
        description={ui(
          "Agents that work for {value0}: what each one is doing, how it is reached, and what is misconfigured.",
          { value0: orgName },
        )}
        meta={
          // A quiet org shows no counters: a strip of zeros says nothing the empty list
          // below does not already say.
          totals &&
          totals.runs === 0 &&
          totals.running_sessions === 0 &&
          agentCounts.attention === 0 ? undefined : (
            <dl className="flex flex-wrap gap-x-5 gap-y-1 text-[13px] text-muted-foreground">
              {/* Agents with work in flight; the org-wide session count is the next fact. */}
              <Fact
                label={ui("Agents running")}
                value={totals ? String(agentCounts.running) : "–"}
              />
              <Fact
                label={ui("Active turns")}
                value={
                  totals
                    ? `${totals.running_sessions} / ${formatCompactNumber(totals.max_active_turns)}`
                    : "–"
                }
              />
              <Fact label={ui("Need attention")} value={String(agentCounts.attention)} />
              <Fact
                label={ui("Runs, 24h")}
                value={totals ? formatCompactNumber(totals.runs) : "–"}
              />
              <Fact label={ui("Failed, 24h")} value={totals ? String(totals.failed) : "–"} />
            </dl>
          )
        }
        actions={
          <>
            <input
              type="file"
              ref={fileInputRef}
              onChange={(event) => {
                const file = event.target.files?.[0];
                if (file) setImportFile(file);
                event.target.value = "";
              }}
              accept=".md,.toml,.yaml,.yml,.json,.zip"
              className="hidden"
              aria-label={ui("Import agent file")}
            />
            <Button variant="outline" onClick={() => fileInputRef.current?.click()}>
              <Upload className="size-4" />
              {ui("Import")}
            </Button>
            <NewAgentLink>
              <Button variant="accent">
                <Plus className="size-4" />
                {ui("New agent")}
              </Button>
            </NewAgentLink>
          </>
        }
      />

      {importFile && (
        <AgentImportDialog
          file={importFile}
          agents={agents ?? []}
          capabilities={allCapabilities ?? []}
          onClose={() => setImportFile(null)}
          onImported={(agent) => router.push(`/agents/${agent.id}`)}
        />
      )}

      <NeedsAttention items={attention} canManage={canManage} />

      {onlyBuiltInAgents && (
        <div className="flex flex-wrap items-center justify-between gap-3 border bg-card p-4">
          <div>
            <p className="text-sm font-medium">
              {ui("Build your first agent with your coding tools")}
            </p>
            <p className="text-xs text-muted-foreground">
              {ui("Connect Claude Code, Codex, Cursor, or Gemini CLI to use Everruns.")}
            </p>
          </div>
          <CodingAgentConnectAction />
        </div>
      )}

      {noAgents ? (
        <FreshOrganization />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex border" role="tablist" aria-label={ui("View")}>
              <ViewButton
                active={view === "agents"}
                onClick={() => setView("agents")}
                icon={<AgentIcon className="size-4" />}
                label={ui("Agents")}
              />
              <ViewButton
                active={view === "channels"}
                onClick={() => setView("channels")}
                icon={<Radio className="size-4" />}
                label={ui("Channels")}
              />
            </div>
            <SearchInput
              placeholder={view === "agents" ? ui("Search agents…") : ui("Search channels…")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              containerClassName="w-64"
            />
            <div className="flex-1" />
            {view === "agents" ? (
              <SectionTabs
                value={agentTab}
                onValueChange={(value) => setAgentTab(value as AgentTab)}
                items={[
                  { value: "all", label: ui("All"), count: agentCounts.all },
                  {
                    value: "running",
                    label: ui("Running"),
                    count: agentCounts.running,
                  },
                  {
                    value: "attention",
                    label: ui("Needs attention"),
                    count: agentCounts.attention,
                  },
                  { value: "idle", label: ui("Idle"), count: agentCounts.idle },
                  {
                    value: "archived",
                    label: ui("Archived"),
                    count: agentCounts.archived,
                  },
                ]}
              />
            ) : (
              <SectionTabs
                value={channelTab}
                onValueChange={(value) => setChannelTab(value as ChannelTab)}
                items={[
                  { value: "all", label: ui("All"), count: channelCounts.all },
                  { value: "live", label: ui("Live"), count: channelCounts.live },
                  {
                    value: "draft",
                    label: ui("Draft"),
                    count: channelCounts.draft,
                  },
                  {
                    value: "paused",
                    label: ui("Paused"),
                    count: channelCounts.paused,
                  },
                  {
                    value: "public",
                    label: ui("Public access"),
                    count: channelCounts.public,
                  },
                ]}
              />
            )}
          </div>

          {view === "agents" ? (
            agentsLoading ? (
              <p className="text-sm text-muted-foreground">{ui("Loading agents…")}</p>
            ) : visibleAgents.length === 0 ? (
              <EmptyState icon={<AgentIcon />} title={ui("No agents match your filters.")} />
            ) : (
              <ul className="divide-y border bg-card">
                {visibleAgents.map((agent) => (
                  <AgentRow
                    key={agent.id}
                    agent={agent}
                    activity={activityById.get(agent.id)}
                    attention={attentionByAgent.get(agent.id) ?? []}
                    activityGeneratedAt={activity?.generated_at}
                    activityWindowStart={activity?.window_start}
                  />
                ))}
              </ul>
            )
          ) : (
            <>
              <ChannelStats
                channels={channels}
                channelActivity={channelActivityById}
                totals={totals}
              />
              {exposuresLoading ? (
                <p className="text-sm text-muted-foreground">{ui("Loading channels…")}</p>
              ) : visibleChannels.length === 0 ? (
                <EmptyState
                  icon={<Radio />}
                  title={channels.length === 0 ? ui("No channels yet") : ui("No channels match.")}
                  description={
                    channels.length === 0
                      ? ui(
                          "Add a channel from an agent's Integrations tab. New channels start as drafts and take no traffic until you publish them.",
                        )
                      : undefined
                  }
                />
              ) : (
                <ul className="divide-y border bg-card">
                  {visibleChannels.map((exposure) => (
                    <ChannelRow
                      key={exposure.channel.id}
                      exposure={exposure}
                      activity={channelActivityById.get(exposure.channel.id)}
                      canManage={canManage}
                    />
                  ))}
                </ul>
              )}
            </>
          )}
        </>
      )}
    </PageContainer>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline gap-1.5">
      <dt>{label}</dt>
      <dd className="font-medium text-foreground">{value}</dd>
    </div>
  );
}

function ViewButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-8 items-center gap-2 px-3 text-[13px] font-medium first:border-r",
        active ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/50",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

function ChannelStats({
  channels,
  channelActivity,
  totals,
}: {
  channels: ReturnType<typeof useOrgExposures>["exposures"];
  channelActivity: Map<string, ChannelActivity>;
  totals: AgentActivityTotals | undefined;
}) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const live = channels.filter((exposure) => channelState(exposure) === "live");
  const busiest = channels
    .map((exposure) => ({
      exposure,
      sessions: channelActivity.get(exposure.channel.id)?.sessions,
    }))
    .filter((entry): entry is { exposure: (typeof channels)[number]; sessions: number } =>
      Boolean(entry.sessions),
    )
    .sort((a, b) => b.sessions - a.sessions)[0];
  const silent = live.filter(
    (exposure) => (channelActivity.get(exposure.channel.id)?.sessions ?? 0) <= 1,
  ).length;
  const fastest = fastestFirstReply(live, channelActivity);

  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
      <StatCard
        label={ui("Sessions via channels")}
        value={totals ? formatCompactNumber(totals.channel_sessions) : "–"}
        hint="Last 7 days"
      />
      <StatCard
        label={ui("People reached")}
        value={totals ? formatCompactNumber(totals.people_reached) : "–"}
        hint="Distinct signed-in or returning callers, 7 days"
      />
      <StatCard
        label={ui("Busiest channel")}
        value={
          busiest ? (
            <span className="text-lg">
              {channelShortName(busiest.exposure.channel.channel_type)}
            </span>
          ) : (
            "–"
          )
        }
        hint={
          busiest
            ? `${busiest.sessions} ${pluralize(busiest.sessions, "session", undefined, displayLocale)}${
                busiest.exposure.agent
                  ? ` · ${getDisplayName(busiest.exposure.agent, displayLocale)}`
                  : ""
              }`
            : "No traffic in 7 days"
        }
      />
      <StatCard
        label={ui("Fastest first reply")}
        value={fastest ? <span className="text-lg">{formatReplyTime(fastest.ms)}</span> : "–"}
        hint={
          fastest
            ? `Median · ${channelShortName(fastest.exposure.channel.channel_type)}${
                fastest.exposure.agent
                  ? ` · ${getDisplayName(fastest.exposure.agent, displayLocale)}`
                  : ""
              }`
            : "No replies on live channels in 7 days"
        }
      />
      <StatCard
        label={ui("Live but silent")}
        value={String(silent)}
        hint="At most one session in 7 days"
      />
    </div>
  );
}

function FreshOrganization() {
  const ui = useUiTranslations();
  const router = useRouter();
  const { data: examples } = useAgentExamples();
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });
  const importExample = useImportAgentExample();
  const [adopting, setAdopting] = useState<string | null>(null);

  const adopt = async (name: string) => {
    setAdopting(name);
    try {
      const agent = await importExample.mutateAsync(name);
      router.push(importedExampleLanding(examples, name, agent.id));
    } finally {
      setAdopting(null);
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <EmptyState
        icon={<AgentIcon />}
        title={ui("No agents yet")}
        description={ui(
          "Create an agent for a job your team repeats. Its channels start as drafts and take no traffic until you publish them.",
        )}
        action={
          <div className="flex flex-wrap justify-center gap-3">
            <NewAgentLink>
              <Button variant="accent">
                <Plus className="size-4" />
                {ui("New agent")}
              </Button>
            </NewAgentLink>
            <CodingAgentConnectAction />
          </div>
        }
      />
      {examples && examples.length > 0 && (
        <section>
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-lg font-semibold tracking-tight">{ui("Start from an example")}</h2>
            <Link
              href="/agents/examples"
              className="inline-flex items-center gap-1 text-[13px] text-muted-foreground hover:text-foreground"
            >
              {ui("All examples")}
              <ArrowRight className="size-3.5" />
            </Link>
          </div>
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {examples.slice(0, EXAMPLE_PREVIEW_LIMIT).map((example) => (
              <ExampleCard
                key={example.name}
                example={example}
                allCapabilities={allCapabilities}
                onImport={adopt}
                adopting={adopting === example.name}
                preview
              />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
