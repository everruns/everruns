"use client";

import { useEffect, useState } from "react";
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

export function MonitoringPage({ scope }: { scope: MonitoringScope }) {
  usePageTitle("Monitoring");
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
        <PageBreadcrumb items={[{ label: "Monitoring" }]} />
        <PageMasthead title="Monitoring" icon={<Activity />} />
        <EmptyState
          icon={<Activity />}
          title={scope === "fleet" ? "Platform access required" : "Monitoring is not enabled"}
          description={
            scope === "fleet"
              ? "Cross-organization monitoring is available to platform operators."
              : "An organization administrator can enable Monitoring in Settings → Features."
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

function MonitoringWorkspace({ scope, org }: { scope: MonitoringScope; org: string }) {
  const [tab, setTab] = useState("execution");
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
  const terminalError =
    snapshot.error instanceof ApiError && [401, 403, 404].includes(snapshot.error.status);
  if (terminalError)
    return (
      <PageContainer>
        <PageMasthead title="Monitoring" icon={<Activity />} />
        <EmptyState
          icon={<AlertTriangle />}
          title="Monitoring is unavailable"
          description={snapshot.error?.message ?? "Access to Monitoring is unavailable."}
        />
      </PageContainer>
    );
  const data =
    frozen ?? (replay && snapshot.data ? replaySnapshot(snapshot.data, replay) : snapshot.data);
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
      <PageBreadcrumb items={[{ label: "Monitoring" }]} />
      <PageMasthead
        title="Monitoring"
        icon={<Activity />}
        badges={<Badge variant="outline">{scope === "fleet" ? "Platform" : "Organization"}</Badge>}
        description={
          scope === "fleet"
            ? "Live execution and worker ownership across all organizations."
            : "Live execution and actor leases within the selected organization."
        }
        meta={
          data && (
            <div className="flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground">
              <span>{number(data.actors)} active sessions</span>
              {scope === "fleet" && (
                <>
                  <span>{number(data.organizations)} organizations</span>
                  <span>{number(replay?.workers ?? data.workers.length)} workers</span>
                </>
              )}
              <span>{number(data.actor_leases)} actor leases</span>
            </div>
          )
        }
        actions={
          <div className="flex items-center gap-2">
            <Badge variant="outline" className={stale && !paused ? "text-warning" : ""}>
              {replay ? "Replay" : frozen ? "Paused" : stale ? "Stale" : "Live"}
            </Badge>
            <Button
              variant="outline"
              size="sm"
              disabled={!snapshot.data}
              onClick={() => (paused ? resume() : setFrozen(snapshot.data!))}
            >
              {paused ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
              {paused ? "Live" : "Pause"}
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
              Replay
            </Button>
          </div>
        }
      />
      <PageControlStrip>
        <SectionTabs
          value={tab}
          onValueChange={setTab}
          aria-label="Monitoring sections"
          items={[
            { value: "execution", label: "Execution" },
            { value: "workers", label: "Workers", disabled: !!replay },
            { value: "actors", label: "Actors", disabled: !!replay },
            { value: "metrics", label: "Metrics" },
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
                  {v === "flow" ? "Flow" : "Density"}
                </Button>
              ))}
            </div>
          ) : tab === "workers" ? (
            <label className="text-xs text-muted-foreground">
              Group size{" "}
              <select
                aria-label="Worker group size"
                value={groupSize}
                onChange={(e) => {
                  setGroupSize(Number(e.target.value));
                  setGroupId(null);
                }}
                className="ml-2 border bg-background px-3 py-1.5 text-foreground"
              >
                <option value={1}>Individual workers</option>
                <option value={8}>8 workers</option>
                <option value={32}>32 workers</option>
              </select>
            </label>
          ) : tab === "actors" ? (
            <div className="flex flex-wrap items-center gap-3 text-xs">
              <span className="text-muted-foreground">
                {worker ? `Holder ${worker}` : "All holders"}
              </span>
              <select
                aria-label="Actor phase filter"
                value={phase ?? ""}
                disabled={paused}
                onChange={(e) => {
                  setPhase(e.target.value || null);
                  setOffset(0);
                }}
                className="border bg-background px-3 py-1.5"
              >
                <option value="">All phases</option>
                {phases.map((p) => (
                  <option key={p.key} value={p.key}>
                    {p.label}
                  </option>
                ))}
              </select>
              {worker && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setWorker(undefined);
                    setOffset(0);
                  }}
                >
                  Clear holder
                </Button>
              )}
            </div>
          ) : (
            <span className="text-xs text-muted-foreground">Last 60 seconds</span>
          )}
          <span className="text-xs text-muted-foreground">
            {data
              ? `${new Date(data.generated_at).toLocaleTimeString()} snapshot`
              : "Waiting for a snapshot"}
          </span>
        </div>
      </PageControlStrip>
      {snapshot.error && !terminalError && (
        <div role="status" className="border border-warning/40 bg-warning/5 p-3 text-sm">
          Updates unavailable. Showing the last successful snapshot; lease validity is unknown until
          refresh succeeds.
        </div>
      )}
      {(data?.cohorts_truncated || data?.workers_truncated) && (
        <div role="status" className="border border-warning/40 p-3 text-sm">
          Population and phase counts are complete; ownership detail exceeds the overview limit and
          is partial.
        </div>
      )}
      {(replay || frozen) && history.length > 1 && (
        <div className="flex flex-wrap items-center gap-4 border bg-card p-3 text-xs">
          <span>{replay ? "Replay" : "Paused"} · observed history</span>
          <input
            type="range"
            aria-label="Replay time"
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
            {new Date(history[0].at).toLocaleTimeString()} –{" "}
            {new Date(history.at(-1)!.at).toLocaleTimeString()}
          </span>
          <Button variant="ghost" size="sm" onClick={resume}>
            Return to live
          </Button>
        </div>
      )}
      {!data ? (
        snapshot.error ? (
          <EmptyState
            icon={<AlertTriangle />}
            title="Monitoring could not load"
            description="Refresh when the service is available."
            action={<Button onClick={() => snapshot.refetch()}>Retry</Button>}
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
                <h2 className="text-sm font-semibold">Lease lifecycle</h2>
                <div className="mt-4 flex flex-wrap items-center gap-x-6 gap-y-3 text-sm">
                  <span>{number(data.actor_leases)} held</span>
                  <span className="text-muted-foreground">→</span>
                  <span className={data.expired_leases ? "text-warning" : ""}>
                    {number(data.expired_leases)} expired, unclaimed
                  </span>
                  <span className="text-muted-foreground">→</span>
                  <span>
                    {number(data.lease_events.reacquired ?? 0)} reacquired / {data.window_seconds}s
                  </span>
                </div>
                <p className="mt-3 text-xs text-muted-foreground">
                  Reacquisition changes ownership of an existing actor; it does not create another
                  turn. Worker heartbeats and actor leases are separate signals.
                </p>
              </section>
            )}
            {actors.error && tab === "actors" && (
              <p role="alert" className="text-sm text-warning">
                Actor ownership could not be refreshed.
              </p>
            )}
          </PageMain>
          <PageRail>
            {actor && !detail.error && detail.data && !replay ? (
              <ActorDetail detail={detail.data} serverNow={serverNow} stale={stale} />
            ) : selectedGroup && tab === "workers" ? (
              <RailSection label="Selected worker group">
                <h2 className="break-all text-sm font-semibold">{selectedGroup.name}</h2>
                <dl className="mt-3 space-y-2 text-xs">
                  <Row label="Workers" value={String(selectedGroup.workers.length)} />
                  <Row label="Actor leases" value={number(selectedGroup.leases)} />
                  <Row
                    label="Reported load"
                    value={
                      selectedGroup.load == null
                        ? "Unavailable in this scope"
                        : `${selectedGroup.load}/${selectedGroup.capacity}`
                    }
                  />
                  <Row label="Stale heartbeats" value={String(selectedGroup.stale)} />
                </dl>
                <h3 className="mt-5 border-t pt-3 text-xs font-semibold">Worker ownership</h3>
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
                        {number(w.actor_leases)} leases{w.status && ` · ${w.status}`}
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
                  Select a worker to inspect its currently leased actors.
                </p>
              </RailSection>
            ) : (
              <RailSection
                label={
                  replay ? "Selected observation" : phase ? "Selected phase" : "Execution pressure"
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
                  <Row label="Oldest runnable" value={duration(data.oldest_runnable_ms)} />
                  <Row label="Expired leases" value={number(data.expired_leases)} />
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
                        Inspect actors →
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setTab("workers")}>
                        Show worker mapping →
                      </Button>
                    </>
                  )}
                </div>
                <p className="mt-4 text-[11px] leading-relaxed text-muted-foreground">
                  {replay
                    ? "Replay contains population summaries. Ownership detail uses live observations and recorded actor history."
                    : "Model phases may retain actor leases while releasing worker slots. Waiting includes input and external tool resolutions."}
                </p>
              </RailSection>
            )}
            {actor && detail.error && (
              <RailSection label="Actor unavailable">
                <p role="alert" className="text-xs">
                  This actor is no longer active or its diagnostic snapshot could not be refreshed.
                </p>
              </RailSection>
            )}
          </PageRail>
        </PageColumns>
      )}
      <PageFooter>
        <span className="text-xs text-muted-foreground">
          {scope === "fleet"
            ? "All organizations · platform-only monitoring"
            : "Selected organization only · adoption feature"}{" "}
          · 1-second refresh · {history.length} observed samples
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
