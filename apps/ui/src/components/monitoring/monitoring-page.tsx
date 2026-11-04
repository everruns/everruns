"use client";
import {
  Select as SharedSelect,
  SelectTrigger as SharedSelectTrigger,
  SelectContent as SharedSelectContent,
  SelectValue as SharedSelectValue,
  SelectItem as SharedSelectItem,
} from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useChatPageRows, useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Activity, AlertTriangle, Pause, Play, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
  EmptyState,
} from "@/components/layout/page-layout";
import { useOrg } from "@/providers/org-provider";
import { useFeatureFlagsState } from "@/providers/feature-flags-provider";
import { usePolicies } from "@/hooks/use-policies";
import { usePageTitle } from "@/hooks/use-page-title";
import { ApiError } from "@/lib/api/client";
import {
  getMonitoring,
  getMonitoringActor,
  getMonitoringActors,
  type MonitoringScope,
} from "@/lib/api/monitoring";
import type { MonitoringSnapshot } from "@/lib/api/types";
import {
  appendHistory,
  historyPoint,
  replaySnapshot,
  phases,
  duration,
  number,
  type HistoryPoint,
} from "./model";
import { ExecutionFlow } from "./execution-flow";
import { ExecutionDensity } from "./execution-density";
import { WorkerGrid, groupWorkers } from "./worker-grid";
import { ActorTable, ActorDetail } from "./actors";
import { MonitoringMetrics } from "./metrics";
import { MonitoringTimelineView } from "./timeline";

export function MonitoringPage({ scope }: { scope: MonitoringScope }) {
  const ui = useUiTranslations();
  usePageTitle(ui("Monitoring"));
  const { currentOrg } = useOrg();
  const flags = useFeatureFlagsState();
  const policy = usePolicies(scope === "fleet" ? "durable" : "monitoring");
  const loading = policy.isLoading || (scope === "organization" && flags.isLoading);
  const allowed =
    scope === "fleet"
      ? policy.can("durable.view")
      : !!flags.flags.monitoring && policy.can("monitoring.view");
  if (loading)
    return (
      <PageContainer>
        <Skeleton className="h-24" />
        <Skeleton className="h-96" />
      </PageContainer>
    );
  if (!allowed)
    return (
      <PageContainer>
        <PageBreadcrumb items={[{ label: ui("Monitoring") }]} />
        <PageMasthead title={ui("Monitoring")} icon={<Activity />} />
        <EmptyState
          icon={<Activity />}
          title={
            scope === "fleet" ? ui("Platform access required") : ui("Monitoring is not enabled")
          }
          description={
            scope === "fleet"
              ? ui("Cross-organization monitoring is available to platform operators.")
              : ui("An organization administrator can enable Monitoring in Settings → Features.")
          }
        />
      </PageContainer>
    );
  // An org switch discards history, frozen views, filters, and actor detail.
  return (
    <MonitoringWorkspace
      key={`${scope}:${currentOrg?.public_id}`}
      scope={scope}
      org={currentOrg?.public_id ?? ""}
    />
  );
}

const MONITORING_TABS = ["execution", "timeline", "workers", "actors", "metrics"];

function MonitoringWorkspace({ scope, org }: { scope: MonitoringScope; org: string }) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  // `?tab=timeline` lets Home link straight to the history it summarises.
  const requestedTab = useSearchParams().get("tab");
  const [tab, setTab] = useState(
    requestedTab && MONITORING_TABS.includes(requestedTab) ? requestedTab : "execution",
  );
  const [view, setView] = useState("flow");
  const [groupSize, setGroupSize] = useState(8);
  const [groupId, setGroupId] = useState<string | null>(null);
  const [worker, setWorker] = useState<string | undefined>();
  const [phase, setPhase] = useState<string | null>(null);
  const [actor, setActor] = useState<string | null>(null);
  const [offset, setOffset] = useState(0);
  const [history, setHistory] = useState<HistoryPoint[]>([]);
  const [frozen, setFrozen] = useState<MonitoringSnapshot | null>(null);
  const [replay, setReplay] = useState<HistoryPoint | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const snapshot = useQuery({
    queryKey: ["monitoring", scope, org],
    queryFn: () => getMonitoring(scope),
    refetchInterval: 1000,
    refetchOnWindowFocus: true,
    retry: false,
  });
  const paused = !!frozen || !!replay;
  const queryClient = useQueryClient();
  useEffect(
    () =>
      queryClient.getQueryCache().subscribe((event) => {
        const key = event.query.queryKey;
        if (
          key[0] === "monitoring" &&
          key[1] === scope &&
          key[2] === org &&
          event.query.state.status === "success"
        ) {
          const observed = queryClient.getQueryData<MonitoringSnapshot>(["monitoring", scope, org]);
          if (observed) setHistory((old) => appendHistory(old, historyPoint(observed)));
        }
      }),
    [queryClient, scope, org],
  );
  useEffect(() => {
    if (paused) return;
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [paused]);
  const actors = useQuery({
    queryKey: ["monitoring-actors", scope, org, offset, worker, phase],
    queryFn: () => getMonitoringActors(scope, offset, worker, phase ?? undefined),
    enabled: tab === "actors" && !paused,
    refetchInterval: paused ? false : 1000,
    retry: false,
  });
  const detail = useQuery({
    queryKey: ["monitoring-actor", scope, org, actor],
    queryFn: () => getMonitoringActor(scope, actor!),
    enabled: !!actor && !paused,
    refetchInterval: paused ? false : 1000,
    retry: false,
  });
  const data =
    frozen ?? (replay && snapshot.data ? replaySnapshot(snapshot.data, replay) : snapshot.data);
  useChatPageFilters({
    tab,
    type: view,
    status: phase,
    selected_id: actor ?? worker ?? groupId,
    offset,
    range: paused ? "paused" : "live",
  });
  useChatPageRows(
    "Monitoring phases",
    (data?.phases ?? []).map((p) => ({ phase: p.phase, count: p.count, oldest_ms: p.oldest_ms })),
    { loading: snapshot.isLoading, error: !!snapshot.error },
  );
  useChatPageRows(
    "Monitoring summary",
    data
      ? [
          {
            actors: data.actors,
            leases: data.actor_leases,
            expired_leases: data.expired_leases,
            organizations: data.organizations,
            generated_at: data.generated_at,
            paused,
          },
        ]
      : [],
    { loading: snapshot.isLoading, error: !!snapshot.error },
  );
  useChatPageRows(
    "Monitoring workers",
    (data?.workers ?? []).map((w) => ({
      id: w.id,
      group: w.worker_group,
      status: w.status,
      capacity: w.capacity,
      current_load: w.current_load,
      actor_leases: w.actor_leases,
    })),
    { enabled: tab === "workers", error: !!snapshot.error },
  );
  useChatPageRows(
    "Monitoring actors",
    (actors.data?.actors ?? []).map((a) => ({
      session_id: a.session_id,
      phase: a.phase,
      lease_state: a.lease_state,
      holder: a.holder,
      fence: a.fence,
      runnable_age_ms: a.runnable_age_ms,
    })),
    { enabled: tab === "actors" && !paused, loading: actors.isLoading, error: !!actors.error },
  );
  const terminalError =
    snapshot.error instanceof ApiError && [401, 403, 404].includes(snapshot.error.status);
  if (terminalError)
    return (
      <PageContainer>
        <PageMasthead title={ui("Monitoring")} icon={<Activity />} />
        <EmptyState
          icon={<AlertTriangle />}
          title={ui("Monitoring is unavailable")}
          description={snapshot.error?.message ?? ui("Access to Monitoring is unavailable.")}
        />
      </PageContainer>
    );
  const stale = !!snapshot.error || clock - snapshot.dataUpdatedAt > 5000;
  const groups = data ? groupWorkers(data.workers, groupSize) : [];
  const selectedGroup = groups.find((g) => g.id === groupId);
  const serverNow = detail.data
    ? Date.parse(detail.data.generated_at) + Math.max(0, clock - detail.dataUpdatedAt)
    : clock;
  const replayIndex = replay ? history.findIndex((h) => h.at === replay.at) : history.length - 1;
  function resume() {
    setFrozen(null);
    setReplay(null);
    setClock(Date.now());
  }
  function selectPhase(value: string) {
    setPhase(value === phase ? null : value);
    setWorker(undefined);
    setActor(null);
    setOffset(0);
  }
  function inspectActors(holder?: string) {
    setWorker(holder);
    setActor(null);
    setOffset(0);
    setTab("actors");
  }
  return (
    <PageContainer fullWidth>
      <PageBreadcrumb items={[{ label: ui("Monitoring") }]} />
      <PageMasthead
        title={ui("Monitoring")}
        icon={<Activity />}
        badges={
          <Badge variant="outline">{scope === "fleet" ? ui("Platform") : ui("Organization")}</Badge>
        }
        description={
          scope === "fleet"
            ? ui("Live execution and worker ownership across all organizations.")
            : ui("Live execution and actor leases within the selected organization.")
        }
        meta={
          data && (
            <div className="flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground">
              <span>
                {number(data.actors)} {ui("active sessions")}
              </span>
              {scope === "fleet" && (
                <>
                  <span>
                    {number(data.organizations)} {ui("organizations")}
                  </span>
                  <span>
                    {number(replay?.workers ?? data.workers.length)} {ui("workers")}
                  </span>
                </>
              )}
              <span>
                {number(data.actor_leases)} {ui("actor leases")}
              </span>
            </div>
          )
        }
        actions={
          <div className="flex items-center gap-2">
            <Badge variant="outline" className={stale && !paused ? "text-warning" : ""}>
              {replay ? ui("Replay") : frozen ? ui("Paused") : stale ? ui("Stale") : ui("Live")}
            </Badge>
            <Button
              variant="outline"
              size="sm"
              disabled={!snapshot.data}
              onClick={() => (paused ? resume() : setFrozen(snapshot.data!))}
            >
              {paused ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
              {paused ? ui("Live") : ui("Pause")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={history.length < 2}
              onClick={() => {
                setReplay(history[0]);
                setFrozen(null);
                setTab("execution");
              }}
            >
              <RotateCcw className="size-3.5" />
              {ui("Replay")}
            </Button>
          </div>
        }
      />
      <PageControlStrip>
        <SectionTabs
          value={tab}
          onValueChange={setTab}
          aria-label={ui("Monitoring sections")}
          items={[
            { value: "execution", label: ui("Execution") },
            { value: "timeline", label: ui("Timeline") },
            { value: "workers", label: ui("Workers"), disabled: !!replay },
            { value: "actors", label: ui("Actors"), disabled: !!replay },
            { value: "metrics", label: ui("Metrics") },
          ]}
        />
        <div className="flex flex-wrap items-center justify-between gap-3 pt-3">
          {tab === "execution" ? (
            <div className="flex gap-1">
              {["flow", "density"].map((v) => (
                <Button
                  key={v}
                  variant="outline"
                  size="sm"
                  aria-pressed={view === v}
                  onClick={() => setView(v)}
                  className={view === v ? "border-primary font-semibold" : "text-muted-foreground"}
                >
                  {v === "flow" ? ui("Flow") : ui("Density")}
                </Button>
              ))}
            </div>
          ) : tab === "timeline" ? (
            <span className="text-xs text-muted-foreground">
              {ui("Recorded history, kept permanently")}
            </span>
          ) : tab === "workers" ? (
            <label className="text-xs text-muted-foreground">
              {ui("Group size")}{" "}
              <SharedSelect
                value={String(groupSize)}
                onValueChange={(selection) => {
                  setGroupSize(Number(selection));
                  setGroupId(null);
                }}
              >
                <SharedSelectTrigger
                  aria-label={ui("Worker group size")}
                  className="ml-2 border bg-background px-3 py-1.5 text-foreground"
                >
                  <SharedSelectValue />
                </SharedSelectTrigger>
                <SharedSelectContent>
                  <SharedSelectItem value={String(1)}>{ui("Individual workers")}</SharedSelectItem>
                  <SharedSelectItem value={String(8)}>{ui("8 workers")}</SharedSelectItem>
                  <SharedSelectItem value={String(32)}>{ui("32 workers")}</SharedSelectItem>
                </SharedSelectContent>
              </SharedSelect>
            </label>
          ) : tab === "actors" ? (
            <div className="flex flex-wrap items-center gap-3 text-xs">
              <span className="text-muted-foreground">
                {worker ? ui("Holder {value0}", { value0: worker }) : ui("All holders")}
              </span>
              <SharedSelect
                value={String(phase ?? "")}
                disabled={paused}
                onValueChange={(selection) => {
                  setPhase(selection || null);
                  setOffset(0);
                }}
              >
                <SharedSelectTrigger
                  aria-label={ui("Actor phase filter")}
                  className="border bg-background px-3 py-1.5"
                >
                  <SharedSelectValue />
                </SharedSelectTrigger>
                <SharedSelectContent>
                  <SharedSelectItem value="">{ui("All phases")}</SharedSelectItem>
                  {phases.map((p) => (
                    <SharedSelectItem key={p.key} value={String(p.key)}>
                      {p.label}
                    </SharedSelectItem>
                  ))}
                </SharedSelectContent>
              </SharedSelect>
              {worker && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setWorker(undefined);
                    setOffset(0);
                  }}
                >
                  {ui("Clear holder")}
                </Button>
              )}
            </div>
          ) : (
            <span className="text-xs text-muted-foreground">{ui("Last 60 seconds")}</span>
          )}
          <span className="text-xs text-muted-foreground">
            {data
              ? ui("{value0} snapshot", {
                  value0: new Date(data.generated_at).toLocaleTimeString(displayLocale),
                })
              : ui("Waiting for a snapshot")}
          </span>
        </div>
      </PageControlStrip>
      {snapshot.error && !terminalError && (
        <div role="status" className="border border-warning/40 bg-warning/5 p-3 text-sm">
          {ui(
            "Updates unavailable. Showing the last successful snapshot; lease validity is unknown until refresh succeeds.",
          )}
        </div>
      )}
      {(data?.cohorts_truncated || data?.workers_truncated) && (
        <div role="status" className="border border-warning/40 p-3 text-sm">
          {ui(
            "Population and phase counts are complete; ownership detail exceeds the overview limit and is partial.",
          )}
        </div>
      )}
      {(replay || frozen) && history.length > 1 && (
        <div className="flex flex-wrap items-center gap-4 border bg-card p-3 text-xs">
          <span>
            {replay ? ui("Replay") : ui("Paused")} {ui("· observed history")}
          </span>
          <Slider
            aria-label={ui("Replay time")}
            className="min-w-32 flex-1 accent-accent"
            min={0}
            max={history.length - 1}
            value={Math.max(0, replayIndex)}
            onChange={(e) => {
              setReplay(history[Number(e.target.value)]);
              setFrozen(null);
              setTab("execution");
            }}
          />
          <span>
            {new Date(history[0].at).toLocaleTimeString(displayLocale)} –{" "}
            {new Date(history.at(-1)!.at).toLocaleTimeString(displayLocale)}
          </span>
          <Button variant="ghost" size="sm" onClick={resume}>
            {ui("Return to live")}
          </Button>
        </div>
      )}
      {tab === "timeline" ? (
        <MonitoringTimelineView scope={scope} />
      ) : !data ? (
        snapshot.error ? (
          <EmptyState
            icon={<AlertTriangle />}
            title={ui("Monitoring could not load")}
            description={ui("Refresh when the service is available.")}
            action={<Button onClick={() => snapshot.refetch()}>{ui("Retry")}</Button>}
          />
        ) : (
          <Skeleton className="h-[500px]" />
        )
      ) : (
        <PageColumns>
          <PageMain>
            <div role="tabpanel" aria-label={tab}>
              {tab === "execution" &&
                (view === "flow" ? (
                  <ExecutionFlow data={data} selected={phase} onSelect={selectPhase} />
                ) : (
                  <ExecutionDensity
                    history={history}
                    selected={replay ? replayIndex : null}
                    onSelect={(i) => {
                      setReplay(history[i]);
                      setFrozen(null);
                    }}
                  />
                ))}
              {tab === "workers" && (
                <WorkerGrid
                  data={data}
                  size={groupSize}
                  selected={groupId}
                  onSelect={(group) => {
                    setGroupId(group.id);
                    setActor(null);
                  }}
                />
              )}
              {tab === "actors" && (
                <ActorTable
                  page={actors.error ? undefined : actors.data}
                  selected={actor}
                  onSelect={setActor}
                  offset={offset}
                  onPage={setOffset}
                  disabled={paused}
                />
              )}
              {tab === "metrics" && (
                <MonitoringMetrics
                  data={data}
                  history={replay ? history.filter((h) => h.at <= replay.at) : history}
                />
              )}
            </div>
            {tab === "execution" && (
              <section className="border bg-card p-4">
                <h2 className="text-sm font-semibold">{ui("Lease lifecycle")}</h2>
                <div className="mt-4 flex flex-wrap items-center gap-x-6 gap-y-3 text-sm">
                  <span>
                    {number(data.actor_leases)} {ui("held")}
                  </span>
                  <span className="text-muted-foreground">→</span>
                  <span className={data.expired_leases ? "text-warning" : ""}>
                    {number(data.expired_leases)} {ui("expired, unclaimed")}
                  </span>
                  <span className="text-muted-foreground">→</span>
                  <span>
                    {number(data.lease_events.reacquired ?? 0)} {ui("reacquired /")}{" "}
                    {data.window_seconds}s
                  </span>
                </div>
                <p className="mt-3 text-xs text-muted-foreground">
                  {ui(
                    "Reacquisition changes ownership of an existing actor; it does not create another turn. Worker heartbeats and actor leases are separate signals.",
                  )}
                </p>
              </section>
            )}
            {actors.error && tab === "actors" && (
              <p role="alert" className="text-sm text-warning">
                {ui("Actor ownership could not be refreshed.")}
              </p>
            )}
          </PageMain>
          <PageRail>
            {actor && !detail.error && detail.data && !replay ? (
              <ActorDetail detail={detail.data} serverNow={serverNow} stale={stale} />
            ) : selectedGroup && tab === "workers" ? (
              <RailSection label={ui("Selected worker group")}>
                <h2 className="break-all text-sm font-semibold">{selectedGroup.name}</h2>
                <dl className="mt-3 space-y-2 text-xs">
                  <Row label={ui("Workers")} value={String(selectedGroup.workers.length)} />
                  <Row label={ui("Actor leases")} value={number(selectedGroup.leases)} />
                  <Row
                    label={ui("Reported load")}
                    value={
                      selectedGroup.load == null
                        ? "Unavailable in this scope"
                        : `${selectedGroup.load}/${selectedGroup.capacity}`
                    }
                  />
                  <Row label={ui("Stale heartbeats")} value={String(selectedGroup.stale)} />
                </dl>
                <h3 className="mt-5 border-t pt-3 text-xs font-semibold">
                  {ui("Worker ownership")}
                </h3>
                <div className="mt-3 max-h-64 space-y-3 overflow-y-auto">
                  {selectedGroup.workers.map((w) => (
                    <div key={w.id} className="text-xs">
                      <button
                        className="break-all text-left font-mono underline-offset-4 hover:underline"
                        onClick={() => inspectActors(w.id)}
                        disabled={paused}
                      >
                        {w.id}
                      </button>
                      <p className="mt-1 text-muted-foreground">
                        {number(w.actor_leases)} {ui("leases")}
                        {w.status && ` · ${w.status}`}
                      </p>
                      <div className="mt-1 space-y-1">
                        {data.ownership
                          .filter((o) => o.worker_id === w.id)
                          .map((o) => (
                            <div key={o.org_id} className="flex justify-between gap-2">
                              <span className="truncate font-mono text-[10px]" title={o.org_id}>
                                {o.org_id}
                              </span>
                              <span>{o.actor_leases}</span>
                            </div>
                          ))}
                      </div>
                    </div>
                  ))}
                </div>
                <p className="mt-4 text-[11px] text-muted-foreground">
                  {ui("Select a worker to inspect its currently leased actors.")}
                </p>
              </RailSection>
            ) : (
              <RailSection
                label={
                  replay
                    ? ui("Selected observation")
                    : phase
                      ? ui("Selected phase")
                      : ui("Execution pressure")
                }
              >
                {phase && (
                  <>
                    <h2 className="text-sm font-semibold">
                      {phases.find((p) => p.key === phase)?.label}
                    </h2>
                    <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                      {phases.find((p) => p.key === phase)?.description}
                    </p>
                  </>
                )}
                <dl className="mt-3 space-y-2 text-xs">
                  {phases.map((p) => (
                    <Row
                      key={p.key}
                      label={p.label}
                      value={number(data.phases.find((r) => r.phase === p.key)?.count ?? 0)}
                    />
                  ))}
                  <Row label={ui("Oldest runnable")} value={duration(data.oldest_runnable_ms)} />
                  <Row label={ui("Expired leases")} value={number(data.expired_leases)} />
                </dl>
                <div className="mt-4 flex flex-col items-start gap-2">
                  {!replay && (
                    <>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={paused}
                        onClick={() => inspectActors()}
                      >
                        {ui("Inspect actors →")}
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setTab("workers")}>
                        {ui("Show worker mapping →")}
                      </Button>
                    </>
                  )}
                </div>
                <p className="mt-4 text-[11px] leading-relaxed text-muted-foreground">
                  {replay
                    ? ui(
                        "Replay contains population summaries. Ownership detail uses live observations and recorded actor history.",
                      )
                    : ui(
                        "Model phases may retain actor leases while releasing worker slots. Waiting includes input and external tool resolutions.",
                      )}
                </p>
              </RailSection>
            )}
            {actor && detail.error && (
              <RailSection label={ui("Actor unavailable")}>
                <p role="alert" className="text-xs">
                  {ui(
                    "This actor is no longer active or its diagnostic snapshot could not be refreshed.",
                  )}
                </p>
              </RailSection>
            )}
          </PageRail>
        </PageColumns>
      )}
      <PageFooter>
        <span className="text-xs text-muted-foreground">
          {scope === "fleet"
            ? ui("All organizations · platform-only monitoring")
            : ui("Selected organization only · adoption feature")}{" "}
          {ui("· 1-second refresh ·")} {history.length} {ui("observed samples")}
        </span>
      </PageFooter>
    </PageContainer>
  );
}
function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right tabular-nums">{value}</dd>
    </div>
  );
}
