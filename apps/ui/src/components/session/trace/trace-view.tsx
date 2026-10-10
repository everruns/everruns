"use client";

// The session Trace tab: control strip, turn rail, virtualized turn list and
// inspector. See knowledge/ui/session-trace.md.
//
// Decision: only loaded turns render, through one virtualizer over flattened
// rows, so a session of any size costs what one window of turns costs. The
// view, the error filter and the selected step live in the URL so a debugging
// link can be shared.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Checkbox } from "@/components/ui/checkbox";
import { Skeleton } from "@/components/ui/skeleton";
import { useSessionTraceOverview, useSessionTraceTurns } from "@/hooks/use-session-trace";
import type { TraceBatch } from "@/lib/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { TraceInspector, type InspectorTarget } from "./trace-inspector";
import {
  BatchRow,
  EmptyTurnRow,
  GapRow,
  StepRow,
  TurnFooter,
  TurnHeader,
  UnloadedBar,
} from "./trace-rows";
import {
  buildRows,
  formatCount,
  parseStepKey,
  selectableKeys,
  stepKey,
  type TraceView as View,
} from "./trace-model";
import { TurnRail } from "./turn-rail";

const VIEWS: { value: View; label: string }[] = [
  { value: "all", label: "Everything" },
  { value: "messages", label: "Messages" },
  { value: "tools", label: "Tool calls" },
];

/** Events that change what the trace shows; deltas never do. */
const TRACE_EVENT_TYPES = new Set([
  "turn.started",
  "turn.completed",
  "turn.failed",
  "turn.cancelled",
  "turn.sealed",
  "llm.generation",
  "tool.started",
  "tool.completed",
]);

export interface TraceViewProps {
  sessionId: string;
  /** Sequence of the newest trace-relevant event seen live, if any. */
  liveSequence?: number;
}

export function TraceView({ sessionId, liveSequence }: TraceViewProps) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const view = (VIEWS.find((v) => v.value === searchParams.get("view"))?.value ?? "all") as View;
  const errorsOnly = searchParams.get("errors") === "1";
  const selectedStep = parseStepKey(searchParams.get("step"));
  const initialTurn = selectedStep?.turn ?? Number(searchParams.get("turn") || 0);

  const setParams = useCallback(
    (changes: Record<string, string | null>) => {
      const next = new URLSearchParams(searchParams.toString());
      for (const [key, value] of Object.entries(changes)) {
        if (value === null) next.delete(key);
        else next.set(key, value);
      }
      const query = next.toString();
      router.replace(query ? `${pathname}?${query}` : pathname, { scroll: false });
    },
    [router, pathname, searchParams],
  );

  const overview = useSessionTraceOverview(sessionId);
  const data = useSessionTraceTurns(sessionId, initialTurn > 0 ? { around: initialTurn } : null);
  const [collapsed, setCollapsed] = useState<Set<number>>(() => new Set());
  const [batch, setBatch] = useState<{ turn: number; batch: TraceBatch } | null>(null);
  const [full, setFull] = useState(false);
  const [pendingTurn, setPendingTurn] = useState<number | null>(initialTurn || null);

  const rows = useMemo(
    () => buildRows(data.turns, data.turnCount, { view, errorsOnly }, collapsed),
    [data.turns, data.turnCount, view, errorsOnly, collapsed],
  );

  const scrollRef = useRef<HTMLDivElement>(null);
  const goToRef = useRef<HTMLInputElement>(null);
  const atBottomRef = useRef(true);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const row = rows[index];
      if (row?.type === "turn") return 96;
      if (row?.type === "step" && row.step.narration) return 88;
      return 56;
    },
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 8,
  });

  // On first load, open at the end of the session, or at the linked turn.
  const openedRef = useRef(false);
  useEffect(() => {
    if (openedRef.current || data.loading || rows.length === 0) return;
    openedRef.current = true;
    if (!pendingTurn) virtualizer.scrollToIndex(rows.length - 1, { align: "end" });
  }, [data.loading, rows.length, pendingTurn, virtualizer]);

  // Scroll to a turn (or the selected step) once its rows exist.
  useEffect(() => {
    if (!pendingTurn) return;
    const key =
      selectedStep && selectedStep.turn === pendingTurn
        ? `s${stepKey(selectedStep.turn, selectedStep.step)}`
        : `t${pendingTurn}`;
    const index = rows.findIndex((r) => r.key === key);
    const headerIndex = index === -1 ? rows.findIndex((r) => r.key === `t${pendingTurn}`) : index;
    if (headerIndex === -1) return;
    virtualizer.scrollToIndex(headerIndex, {
      align: index === headerIndex && key.startsWith("s") ? "center" : "start",
    });
    setPendingTurn(null);
  }, [pendingTurn, rows, selectedStep, virtualizer]);

  // Live: refresh the tail when new trace events arrive and the tail is loaded.
  const lastLoaded = data.turns[data.turns.length - 1]?.turn ?? 0;
  const tailLoaded = lastLoaded >= data.turnCount;
  const { refreshTail } = data;
  useEffect(() => {
    if (liveSequence === undefined || data.loading) return;
    const timer = setTimeout(() => {
      if (tailLoaded) void refreshTail();
      else void overview.refetch();
    }, 600);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- refresh on new events only
  }, [liveSequence]);

  // Follow the bottom while a session runs, if the reader is already there.
  useEffect(() => {
    if (atBottomRef.current && tailLoaded && rows.length > 0 && openedRef.current) {
      virtualizer.scrollToIndex(rows.length - 1, { align: "end" });
    }
  }, [rows.length, tailLoaded, virtualizer, data.turns]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "g") {
        e.preventDefault();
        goToRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const jump = useCallback(
    (turn: number) => {
      setPendingTurn(turn);
      void data.jumpTo(turn);
    },
    [data],
  );

  const selectStep = useCallback(
    (turn: number, step: number) => {
      setBatch(null);
      setFull(false);
      setParams({ step: stepKey(turn, step) });
    },
    [setParams],
  );

  const keys = useMemo(() => selectableKeys(rows), [rows]);
  const currentKey = selectedStep ? stepKey(selectedStep.turn, selectedStep.step) : null;
  const position = currentKey ? keys.indexOf(currentKey) : -1;
  const move = (delta: number) => {
    const next = parseStepKey(keys[position + delta]);
    if (!next) return;
    selectStep(next.turn, next.step);
    const index = rows.findIndex((r) => r.key === `s${stepKey(next.turn, next.step)}`);
    if (index !== -1) virtualizer.scrollToIndex(index, { align: "auto" });
  };

  const target: InspectorTarget | null = batch
    ? { type: "batch", turn: batch.turn, batch: batch.batch }
    : selectedStep
      ? { type: "step", ...selectedStep }
      : null;

  const firstLoaded = data.turns[0]?.turn ?? 0;
  const earlierError = overview.data?.error_turns.find((t) => t < firstLoaded);
  // Steps before the window are known exactly only when nothing follows it.
  const earlierSteps = useMemo(() => {
    if (!overview.data || !firstLoaded || !tailLoaded) return undefined;
    const loadedSteps = data.turns.reduce((sum, t) => sum + t.step_count, 0);
    return Math.max(0, overview.data.step_count - loadedSteps);
  }, [overview.data, data.turns, firstLoaded, tailLoaded]);

  const activeTurn = selectedStep?.turn ?? batch?.turn ?? null;
  const totals = overview.data;

  return (
    <section aria-label="Session trace" className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-3 border-b bg-card px-6 py-3">
        <div role="radiogroup" aria-label="View" className="flex border">
          {VIEWS.map((option) => (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={view === option.value}
              onClick={() => setParams({ view: option.value === "all" ? null : option.value })}
              className={cn(
                "h-7 px-3 text-xs font-medium",
                view === option.value
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:bg-muted",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
        <label className="flex items-center gap-2 text-xs">
          <Checkbox
            checked={errorsOnly}
            onCheckedChange={(checked) => setParams({ errors: checked ? "1" : null })}
          />
          Errors only
        </label>
        {totals && (
          <div className="ml-auto font-mono text-[11px] text-muted-foreground">
            {formatCount(totals.turn_count)} turns · {formatCount(totals.step_count)} steps
            {totals.error_count > 0 && (
              <span className="text-destructive"> · {formatCount(totals.error_count)} errors</span>
            )}
          </div>
        )}
      </div>

      <div className="bg-brand-dots grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_clamp(320px,32vw,420px)] min-[1280px]:grid-cols-[232px_minmax(0,1fr)_clamp(320px,32vw,420px)]">
        <aside className="hidden min-h-0 overflow-y-auto min-[1280px]:block" aria-label="Turns">
          <TurnRail
            overview={overview.data}
            turns={data.turns}
            activeTurn={activeTurn}
            onJump={jump}
            onScrollTo={(turn) => {
              const index = rows.findIndex((r) => r.key === `t${turn}`);
              if (index !== -1)
                virtualizer.scrollToIndex(index, { align: "start", behavior: "smooth" });
            }}
            goToRef={goToRef}
          />
        </aside>

        <div
          ref={scrollRef}
          className="min-h-0 overflow-y-auto px-3 py-4 sm:px-6 sm:py-5"
          onScroll={(e) => {
            const el = e.currentTarget;
            atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
          }}
        >
          {data.loading ? (
            <div className="space-y-3">
              <Skeleton className="h-20 w-full" />
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
            </div>
          ) : data.error ? (
            <div className="border bg-card p-4 text-sm text-destructive">
              Could not load the trace: {data.error.message}
            </div>
          ) : rows.length === 0 ? (
            <div className="border bg-card p-6 text-sm text-muted-foreground">
              No turns yet. Turns appear here as the session runs.
            </div>
          ) : (
            <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
              {virtualizer.getVirtualItems().map((item) => {
                const row = rows[item.index];
                if (!row) return null;
                return (
                  <div
                    key={item.key}
                    data-index={item.index}
                    ref={virtualizer.measureElement}
                    className="absolute top-0 left-0 w-full"
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    {row.type === "unloaded" && (
                      <div className={row.edge === "earlier" ? "mb-5" : "mt-5"}>
                        <UnloadedBar
                          edge={row.edge}
                          from={row.from}
                          to={row.to}
                          steps={row.edge === "earlier" ? earlierSteps : undefined}
                          errorTurns={
                            row.edge === "earlier"
                              ? totals?.error_turns.filter((t) => t < firstLoaded).length
                              : undefined
                          }
                          busy={data.busy}
                          onLoad={() =>
                            void (row.edge === "earlier" ? data.loadEarlier() : data.loadLater())
                          }
                          onJumpToError={
                            row.edge === "earlier" && earlierError
                              ? () => jump(earlierError)
                              : undefined
                          }
                        />
                      </div>
                    )}
                    {row.type === "turn" && (
                      <TurnHeader
                        turn={row.turn}
                        collapsed={row.collapsed}
                        showScale
                        onToggle={() =>
                          setCollapsed((current) => {
                            const next = new Set(current);
                            if (next.has(row.turn.turn)) next.delete(row.turn.turn);
                            else next.add(row.turn.turn);
                            return next;
                          })
                        }
                      />
                    )}
                    {row.type === "step" && (
                      <StepRow
                        turn={row.turn}
                        step={row.step}
                        view={view}
                        selected={
                          !batch &&
                          selectedStep?.turn === row.turn.turn &&
                          selectedStep.step === row.step.step
                        }
                        onSelect={() => selectStep(row.turn.turn, row.step.step)}
                      />
                    )}
                    {row.type === "batch" && (
                      <BatchRow
                        turn={row.turn}
                        batch={row.batch}
                        selected={
                          batch?.turn === row.turn.turn &&
                          batch.batch.first_step === row.batch.first_step
                        }
                        onSelect={() => {
                          setBatch({ turn: row.turn.turn, batch: row.batch });
                          setParams({ step: null });
                        }}
                      />
                    )}
                    {row.type === "gap" && (
                      <GapRow
                        gap={row.gap}
                        busy={data.busy}
                        onShowMore={() => void data.expandGap(row.turn.turn, row.gap)}
                      />
                    )}
                    {row.type === "empty" && <EmptyTurnRow />}
                    {row.type === "footer" && <TurnFooter turn={row.turn} />}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Below lg the inspector opens over the list, only while a step is selected. */}
        <aside
          className={cn(
            "min-h-0 overflow-y-auto bg-card lg:static lg:z-auto lg:block lg:border-l",
            target ? "fixed inset-0 z-40 block" : "hidden",
          )}
          aria-label="Step inspector"
        >
          {target && (
            <div className="sticky top-0 z-10 flex justify-end border-b bg-card px-3 py-2 lg:hidden">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setBatch(null);
                  setParams({ step: null });
                }}
              >
                Back to trace
              </Button>
            </div>
          )}
          <TraceInspector
            sessionId={sessionId}
            target={target}
            full={full}
            onLoadFull={() => setFull(true)}
            onPrev={position > 0 ? () => move(-1) : undefined}
            onNext={position !== -1 && position < keys.length - 1 ? () => move(1) : undefined}
          />
        </aside>
      </div>
    </section>
  );
}

/** Newest sequence among live events that change the trace. */
export function latestTraceSequence(
  events: { type: string; sequence?: number | null }[] | undefined,
): number | undefined {
  if (!events) return undefined;
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const event = events[i];
    if (event && TRACE_EVENT_TYPES.has(event.type) && event.sequence != null) return event.sequence;
  }
  return undefined;
}
