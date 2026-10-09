"use client";

// Agents home (flag `agents_home`): the Agents page as the place where a team
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
  matchesAgentTab,
  sortAgents,
  sortChannels,
  type AgentTab,
  type AttentionItem,
  type ChannelState,
} from "@/lib/agents-home";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { formatCompactNumber, pluralize } from "@/lib/formatting";
import type { AgentActivity, ChannelActivity } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { AgentRow } from "./agent-row";
import { ChannelRow } from "./channel-row";
import { NeedsAttention } from "./needs-attention";

type View = "agents" | "channels";
type ChannelTab = "all" | ChannelState | "public";

const EXAMPLE_PREVIEW_LIMIT = 3;

export function AgentsHome() {
  usePageTitle("Agents");
  const router = useRouter();
  const searchParams = useSearchParams();
  const view: View = searchParams.get("view") === "channels" ? "channels" : "agents";
  const { currentOrg } = useOrg();
  const { can } = usePolicies("agents");
  const canManage = can("agent.manage");

  const { data: agents, isLoading: agentsLoading } = useAgents({ includeArchived: true });
  const { data: activity } = useAgentActivity();
  const { exposures, isLoading: exposuresLoading } = useOrgExposures();
  const { data: healthIssues } = useHealthIssues();
  const { data: models } = useModels();
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });

  const [search, setSearch] = useState("");
  const [agentTab, setAgentTab] = useState<AgentTab>("all");
  const [channelTab, setChannelTab] = useState<ChannelTab>("all");

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
      }),
    [agents, channels, healthIssues, models],
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
      return [agent.display_name, agent.name, agent.description]
        .filter(Boolean)
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
    return sortAgents(filtered, activityById, attentionByAgent);
  }, [agents, agentTab, search, activityById, attentionByAgent]);

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
        exposure.agent ? getDisplayName(exposure.agent) : "",
      ]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
    return sortChannels(filtered, sessionsByChannel);
  }, [channels, channelTab, search, sessionsByChannel]);

  const totals = activity?.totals;
  const orgName = currentOrg?.name ?? "this organization";
  const noAgents = !agentsLoading && (agents ?? []).length === 0;

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Agents" }]} />

      <PageMasthead
        icon={<AgentIcon />}
        title="Agents"
        description={`Agents that work for ${orgName}: what each one is doing, how it is reached, and what is misconfigured.`}
        meta={
          <dl className="flex flex-wrap gap-x-5 gap-y-1 text-[13px] text-muted-foreground">
            {/* Agents with work in flight; the org-wide session count is the next fact. */}
            <Fact label="Agents running" value={totals ? String(agentCounts.running) : "–"} />
            <Fact
              label="Active turns"
              value={
                totals
                  ? `${totals.running_sessions} / ${formatCompactNumber(totals.max_active_turns)}`
                  : "–"
              }
            />
            <Fact label="Need attention" value={String(agentCounts.attention)} />
            <Fact label="Runs, 24h" value={totals ? formatCompactNumber(totals.runs) : "–"} />
            <Fact label="Failed, 24h" value={totals ? String(totals.failed) : "–"} />
          </dl>
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
              aria-label="Import agent file"
            />
            <Button variant="outline" onClick={() => fileInputRef.current?.click()}>
              <Upload className="size-4" />
              Import
            </Button>
            <NewAgentLink>
              <Button variant="accent">
                <Plus className="size-4" />
                New agent
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

      {noAgents ? (
        <FreshOrganization />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex border" role="tablist" aria-label="View">
              <ViewButton
                active={view === "agents"}
                onClick={() => setView("agents")}
                icon={<AgentIcon className="size-4" />}
                label="Agents"
              />
              <ViewButton
                active={view === "channels"}
                onClick={() => setView("channels")}
                icon={<Radio className="size-4" />}
                label="Channels"
              />
            </div>
            <SearchInput
              placeholder={view === "agents" ? "Search agents…" : "Search channels…"}
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
                  { value: "all", label: "All", count: agentCounts.all },
                  { value: "running", label: "Running", count: agentCounts.running },
                  { value: "attention", label: "Needs attention", count: agentCounts.attention },
                  { value: "idle", label: "Idle", count: agentCounts.idle },
                  { value: "archived", label: "Archived", count: agentCounts.archived },
                ]}
              />
            ) : (
              <SectionTabs
                value={channelTab}
                onValueChange={(value) => setChannelTab(value as ChannelTab)}
                items={[
                  { value: "all", label: "All", count: channelCounts.all },
                  { value: "live", label: "Live", count: channelCounts.live },
                  { value: "draft", label: "Draft", count: channelCounts.draft },
                  { value: "paused", label: "Paused", count: channelCounts.paused },
                  { value: "public", label: "Public access", count: channelCounts.public },
                ]}
              />
            )}
          </div>

          {view === "agents" ? (
            agentsLoading ? (
              <p className="text-sm text-muted-foreground">Loading agents…</p>
            ) : visibleAgents.length === 0 ? (
              <EmptyState icon={<AgentIcon />} title="No agents match your filters." />
            ) : (
              <ul className="divide-y border bg-card">
                {visibleAgents.map((agent) => (
                  <AgentRow
                    key={agent.id}
                    agent={agent}
                    activity={activityById.get(agent.id)}
                    attention={attentionByAgent.get(agent.id) ?? []}
                  />
                ))}
              </ul>
            )
          ) : (
            <>
              <ChannelStats
                channels={channels}
                channelActivity={channelActivityById}
                channelSessions={totals?.channel_sessions}
              />
              {exposuresLoading ? (
                <p className="text-sm text-muted-foreground">Loading channels…</p>
              ) : visibleChannels.length === 0 ? (
                <EmptyState
                  icon={<Radio />}
                  title={channels.length === 0 ? "No channels yet" : "No channels match."}
                  description={
                    channels.length === 0
                      ? "Add a channel from an agent's Integrations tab. New channels start as drafts and take no traffic until you publish them."
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
  channelSessions,
}: {
  channels: ReturnType<typeof useOrgExposures>["exposures"];
  channelActivity: Map<string, ChannelActivity>;
  channelSessions: number | undefined;
}) {
  const live = channels.filter((exposure) => channelState(exposure) === "live");
  const busiest = channels
    .map((exposure) => ({ exposure, sessions: channelActivity.get(exposure.channel.id)?.sessions }))
    .filter((entry): entry is { exposure: (typeof channels)[number]; sessions: number } =>
      Boolean(entry.sessions),
    )
    .sort((a, b) => b.sessions - a.sessions)[0];
  const silent = live.filter(
    (exposure) => (channelActivity.get(exposure.channel.id)?.sessions ?? 0) <= 1,
  ).length;
  const publicLive = channels.filter((exposure) => exposure.publiclyReachable).length;

  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
      <StatCard
        label="Sessions via channels"
        value={channelSessions === undefined ? "–" : formatCompactNumber(channelSessions)}
        hint="Last 7 days"
      />
      <StatCard
        label="Live channels"
        value={String(live.length)}
        hint={`of ${channels.length} ${pluralize(channels.length, "channel")}`}
      />
      <StatCard
        label="Busiest channel"
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
            ? `${busiest.sessions} ${pluralize(busiest.sessions, "session")}${
                busiest.exposure.agent ? ` · ${getDisplayName(busiest.exposure.agent)}` : ""
              }`
            : "No traffic in 7 days"
        }
      />
      <StatCard
        label="Public access"
        value={String(publicLive)}
        hint="Live with no sign-in"
        className={publicLive > 0 ? "text-destructive" : undefined}
      />
      <StatCard
        label="Live but silent"
        value={String(silent)}
        hint="At most one session in 7 days"
      />
    </div>
  );
}

function FreshOrganization() {
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
        title="No agents yet"
        description="Create an agent for a job your team repeats. Its channels start as drafts and take no traffic until you publish them."
        action={
          <NewAgentLink>
            <Button variant="accent">
              <Plus className="size-4" />
              New agent
            </Button>
          </NewAgentLink>
        }
      />
      {examples && examples.length > 0 && (
        <section>
          <div className="mb-4 flex items-center justify-between">
            <h2 className="text-lg font-semibold tracking-tight">Start from an example</h2>
            <Link
              href="/agents/examples"
              className="inline-flex items-center gap-1 text-[13px] text-muted-foreground hover:text-foreground"
            >
              All examples
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
