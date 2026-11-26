"use client";
import { SegmentedControl } from "@/components/ui/segmented-control";

// The session Trace tab: control strip, turn rail, virtualized turn list and
// inspector. See knowledge/ui/session-trace.md.
//
// Decision: only loaded turns render, through one virtualizer over flattened
// rows, so a session of any size costs what one window of turns costs. The
// view, the error filter and the selected step live in the URL so a debugging
// link can be shared.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Checkbox } from "@/components/ui/checkbox";
import { Skeleton } from "@/components/ui/skeleton";
import {
  findTraceSequence,
  useSessionTraceOverview,
  useSessionTraceTurns,
  useTraceExpansions,
  useTraceLifecycle,
} from "@/hooks/use-session-trace";
import type { TraceBatch } from "@/lib/api/types";
import { Button } from "@/components/ui/button";
import { List, SlidersHorizontal } from "lucide-react";
import { Drawer, DrawerContent, DrawerTitle } from "@/components/ui/drawer";
import "./trace-layout.css";
import { TraceInspector, type InspectorTarget, type RequestTarget } from "./trace-inspector";
import {
  BatchRow,
  EmptyTurnRow,
  ExpansionFooter,
  GapRow,
  LifecycleRow,
  StepRow,
  TurnFooter,
  TurnHeader,
  UnloadedBar,
} from "./trace-rows";
import {
  buildRows,
  formatCount,
  parseStepKey,
  rowIndexOfStep,
  selectableKeys,
  stepHoldingSequence,
  stepKey,
  type TraceView as View,
  reactionsByTurn,
} from "./trace-model";
import { useMessageReactionList } from "@/hooks/use-message-reactions";
import { TraceSearch } from "./trace-search";
import { TurnRail } from "./turn-rail";
import { DreamingTrace } from "./dreaming-trace";
import { TurnInputInspector } from "./turn-input-inspector";
import { RequestSheet } from "./request-sheet";

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
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const view = (VIEWS.find((v) => v.value === searchParams.get("view"))?.value ?? "all") as View;
  const errorsOnly = searchParams.get("errors") === "1";
  const showLifecycle = searchParams.get("lifecycle") === "1";
  const plainText = searchParams.get("text") === "plain";
  const selectedStep = parseStepKey(searchParams.get("step"));
  const failureSequence = Number(searchParams.get("failure") || 0);
  const linkedFailure =
    Number.isSafeInteger(failureSequence) && failureSequence > 0 ? failureSequence : 0;
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
  const data = useSessionTraceTurns(
    sessionId,
    linkedFailure ? { sequence: linkedFailure } : initialTurn > 0 ? { around: initialTurn } : null,
  );
  const failedTurn = data.turns.find(
    (turn) =>
      linkedFailure >= turn.start_sequence && linkedFailure <= (turn.end_sequence ?? Infinity),
  );
  const [collapsed, setCollapsed] = useState<Set<number>>(() => new Set());
  const [batch, setBatch] = useState<{ turn: number; batch: TraceBatch } | null>(null);
  const [turnInput, setTurnInput] = useState<number | null>(null);
  const [full, setFull] = useState(false);
  const [request, setRequest] = useState<RequestTarget | null>(null);
  // A sub-agent's step opened inline; its numbers belong to the child session,
  // so it lives in state rather than the URL.
  const [childStep, setChildStep] = useState<{
    sessionId: string;
    turn: number;
    step: number;
  } | null>(null);
  const [pending, setPending] = useState<{ turn: number; step?: number } | null>(
    initialTurn ? { turn: initialTurn, step: selectedStep?.step } : null,
  );
  const expansions = useTraceExpansions(sessionId);
  const openTurns = useMemo(
    () => data.turns.filter((t) => !collapsed.has(t.turn)),
    [data.turns, collapsed],
  );
  const lifecycle = useTraceLifecycle(sessionId, openTurns, showLifecycle);
  const reactionList = useMessageReactionList(sessionId);
  const turnReactions = useMemo(
    () => reactionsByTurn(data.turns, reactionList.data),
    [data.turns, reactionList.data],
  );

  const rows = useMemo(
    () =>
      buildRows(data.turns, data.turnCount, { view, errorsOnly }, collapsed, {
        expanded: expansions.expanded,
        lifecycle: showLifecycle ? lifecycle : undefined,
      }),
    [
      data.turns,
      data.turnCount,
      view,
      errorsOnly,
      collapsed,
      expansions.expanded,
      lifecycle,
      showLifecycle,
    ],
  );

  const paneRef = useRef<HTMLElement>(null);
  const [wide, setWide] = useState(false);
  const [railVisible, setRailVisible] = useState(false);
  const [turnsOpen, setTurnsOpen] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  useEffect(() => {
    const pane = paneRef.current;
    if (!pane) return;
    const observer = new ResizeObserver(([entry]) => {
      setWide(entry.contentRect.width >= 920);
      setRailVisible(entry.contentRect.width >= 1120);
    });
    observer.observe(pane);
    return () => observer.disconnect();
  }, []);

  const scrollRef = useRef<HTMLDivElement>(null);
  const goToRef = useRef<HTMLInputElement>(null);
  const atBottomRef = useRef(true);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const row = rows[index];
      if (row?.type === "turn") return 96;
      if ((row?.type === "step" || row?.type === "member") && row.step.narration) return 88;
      if (row?.type === "lifecycle") return 28;
      if (row?.type === "more") return 36;
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
    if (!pending && !linkedFailure) virtualizer.scrollToIndex(rows.length - 1, { align: "end" });
  }, [data.loading, rows.length, pending, virtualizer, linkedFailure]);

  // Scroll to a turn (or a step in it) once its rows exist.
  //
  // Decision: the scroll waits one frame. When a far jump replaces the loaded
  // window, the list's height changes in the same commit; scrolling before the
  // browser lays that out leaves the reader where they were, clamped to the
  // old height, so a minimap click looked like it did nothing.
  const scrollFrameRef = useRef<number | null>(null);
  useEffect(() => {
    if (!pending) return;
    const headerIndex = rows.findIndex((r) => r.key === `t${pending.turn}`);
    if (headerIndex === -1) return;
    const stepIndex =
      pending.step !== undefined ? rowIndexOfStep(rows, pending.turn, pending.step) : -1;
    if (scrollFrameRef.current !== null) cancelAnimationFrame(scrollFrameRef.current);
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null;
      if (stepIndex !== -1) virtualizer.scrollToIndex(stepIndex, { align: "center" });
      else virtualizer.scrollToIndex(headerIndex, { align: "start" });
    });
    setPending(null);
  }, [pending, rows, virtualizer]);
  useEffect(
    () => () => {
      if (scrollFrameRef.current !== null) cancelAnimationFrame(scrollFrameRef.current);
    },
    [],
  );

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

  const jump = useCallback(
    (turn: number, step?: number) => {
      setTurnsOpen(false);
      setPending({ turn, step });
      void data.jumpTo(turn);
    },
    [data],
  );

  const selectStep = useCallback(
    (turn: number, step: number) => {
      setBatch(null);
      setChildStep(null);
      setTurnInput(null);
      setFull(false);
      setParams({ step: stepKey(turn, step) });
    },
    [setParams],
  );

  const clearSelection = useCallback(() => {
    setBatch(null);
    setChildStep(null);
    setTurnInput(null);
    setParams({ step: null });
  }, [setParams]);

  const keys = useMemo(() => selectableKeys(rows), [rows]);
  const currentKey =
    selectedStep && !childStep ? stepKey(selectedStep.turn, selectedStep.step) : null;
  const position = currentKey ? keys.indexOf(currentKey) : -1;
  const move = (delta: number) => {
    // With nothing selected, j starts at the first step and k at the last.
    const index = position === -1 ? (delta > 0 ? 0 : keys.length - 1) : position + delta;
    const next = parseStepKey(keys[index]);
    if (!next) return;
    selectStep(next.turn, next.step);
    const rowIndex = rowIndexOfStep(rows, next.turn, next.step);
    if (rowIndex !== -1) virtualizer.scrollToIndex(rowIndex, { align: "auto" });
  };
  const moveRef = useRef(move);
  moveRef.current = move;

  // Keyboard: j/k walk steps, Escape closes the inspector, ⌘G focuses go-to.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A request modal owns its keys, including the go-to-turn shortcut.
      if (document.querySelector("[data-slot='dialog-content']")) return;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "g") {
        e.preventDefault();
        if (!railVisible) setTurnsOpen(true);
        requestAnimationFrame(() => goToRef.current?.focus());
        return;
      }
      const el = e.target as HTMLElement | null;
      const typing =
        !!el &&
        (el.tagName === "INPUT" ||
          el.tagName === "TEXTAREA" ||
          el.tagName === "SELECT" ||
          el.isContentEditable);
      if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "j" || e.key === "k") {
        e.preventDefault();
        moveRef.current(e.key === "j" ? 1 : -1);
      } else if (e.key === "Escape") {
        clearSelection();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [clearSelection, railVisible]);

  // A search hit: load its turn and select the step that holds it, if loaded.
  const goToSequence = useCallback(
    async (sequence: number) => {
      const turn = await findTraceSequence(sessionId, sequence);
      if (!turn) return;
      const step = stepHoldingSequence(turn, sequence);
      if (step) selectStep(turn.turn, step.step);
      jump(turn.turn, step?.step);
    },
    [sessionId, selectStep, jump],
  );

  // A terminal failure often falls after its model/tool step's event range.
  // Select the last recorded failed step rather than treating the terminal
  // event as if it were itself an assistant message or tool result.
  const openedFailure = useRef(0);
  const jumpToFailure = data.jumpTo;
  useEffect(() => {
    if (!linkedFailure || data.loading || openedFailure.current === linkedFailure) return;
    openedFailure.current = linkedFailure;
    const open = async () => {
      const turn = failedTurn ?? (await findTraceSequence(sessionId, linkedFailure));
      if (!turn || turn.status !== "failed") {
        // The terminal event can arrive before the asynchronous trace index.
        // Allow a later trace refresh to resolve and select the failure.
        openedFailure.current = 0;
        return;
      }
      let step: number | undefined;
      for (const item of turn.items) {
        if (item.type === "step" && item.status === "error") step = item.step;
        if (item.type === "batch" && item.failures.length) step = item.failures.at(-1)?.step;
      }
      setParams({ turn: String(turn.turn), step: step ? stepKey(turn.turn, step) : null });
      setPending({ turn: turn.turn, step });
      if (!failedTurn) await jumpToFailure(turn.turn);
    };
    void open().catch(() => {
      openedFailure.current = 0;
    });
  }, [linkedFailure, data.loading, failedTurn, sessionId, setParams, jumpToFailure]);

  const target: InspectorTarget | null = batch
    ? { type: "batch", turn: batch.turn, batch: batch.batch }
    : childStep
      ? { type: "step", ...childStep }
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

  const activeTurn = turnInput ?? (childStep ? null : selectedStep?.turn) ?? batch?.turn ?? null;
  const inspectorOpen = !!(turnInput || target);
  // Turns with rows on screen, for the minimap's "in view" marker.
  const visibleItems = virtualizer.getVirtualItems();
  const scrollTop = virtualizer.scrollOffset ?? 0;
  const viewportHeight = virtualizer.scrollRect?.height ?? 0;
  const visibleTurns = new Set<number>();
  for (const item of visibleItems) {
    if (item.end <= scrollTop || item.start >= scrollTop + viewportHeight) continue;
    const row = rows[item.index];
    if (row && row.type !== "unloaded") visibleTurns.add(row.turn.turn);
  }
  const totals = overview.data;

  const rail = (
    <TurnRail
      overview={overview.data}
      turns={data.turns}
      activeTurn={activeTurn}
      visibleTurns={visibleTurns}
      onJump={jump}
      goToRef={goToRef}
      onScrollTo={(turn) => {
        setTurnsOpen(false);
        const index = rows.findIndex((r) => r.key === `t${turn}`);
        if (index !== -1) virtualizer.scrollToIndex(index, { align: "start", behavior: "smooth" });
      }}
    />
  );
  const inspector = turnInput ? (
    <TurnInputInspector sessionId={sessionId} turn={turnInput} onClose={clearSelection} />
  ) : (
    <TraceInspector
      sessionId={sessionId}
      target={target}
      full={full}
      onLoadFull={() => setFull(true)}
      onRequest={setRequest}
      onPrev={position > 0 ? () => move(-1) : undefined}
      onNext={position !== -1 && position < keys.length - 1 ? () => move(1) : undefined}
    />
  );

  return (
    <section
      ref={paneRef}
      aria-label={ui("Session trace")}
      className="trace-pane flex min-h-0 min-w-0 flex-1 flex-col"
    >
      <DreamingTrace sessionId={sessionId} />
      {linkedFailure > 0 && !data.loading && (
        <div
          className="border-b border-destructive/30 bg-destructive/5 px-6 py-3 text-sm"
          role="status"
        >
          {failedTurn?.status === "failed" ? (
            <>
              <span className="font-medium text-destructive">
                {ui("Turn")} {failedTurn.turn} {ui("failed")}
              </span>
              {failedTurn.ended_at && (
                <span className="ml-3 text-xs text-muted-foreground">
                  {new Date(failedTurn.ended_at).toLocaleString(displayLocale)}
                </span>
              )}
              <p className="mt-1 break-words">
                {failedTurn.error?.trim() || ui("No error details recorded")}
              </p>
            </>
          ) : (
            <p>
              {ui(
                "The recorded failure could not be located in this trace. Check the Events view for the terminal event.",
              )}
            </p>
          )}
        </div>
      )}
      <div className="trace-toolbar border-b bg-card px-3 py-2">
        <div className="trace-primary-controls flex min-w-0 flex-wrap items-center gap-2">
          <Button
            className="trace-turns-trigger"
            aria-label={ui("Turns")}
            title={ui("Turns")}
            variant="outline"
            size="sm"
            onClick={() => setTurnsOpen(true)}
          >
            <List />
            <span className="trace-control-label">{ui("Turns")}</span>
          </Button>
          <SegmentedControl
            label={ui("View")}
            value={view}
            onValueChange={(value) => setParams({ view: value === "all" ? null : value })}
            options={VIEWS}
          />
          <Button
            variant="outline"
            size="sm"
            className="trace-filters-trigger ml-auto"
            aria-label={ui("Filters")}
            title={ui("Filters")}
            aria-expanded={filtersOpen}
            onClick={() => setFiltersOpen(!filtersOpen)}
          >
            <SlidersHorizontal />
            <span className="trace-control-label">{ui("Filters")}</span>
            {(errorsOnly || showLifecycle || plainText) && <span aria-hidden>•</span>}
          </Button>
        </div>
        <div
          className="trace-secondary-controls flex flex-wrap items-center gap-3"
          data-open={filtersOpen}
        >
          <SegmentedControl
            label={ui("Message format")}
            value={plainText ? "plain" : "markdown"}
            onValueChange={(value) => setParams({ text: value === "plain" ? "plain" : null })}
            options={[
              { value: "markdown", label: ui("Markdown") },
              { value: "plain", label: ui("Plain text") },
            ]}
          />
          <label className="flex items-center gap-2 text-xs">
            <Checkbox
              checked={errorsOnly}
              onCheckedChange={(checked) => setParams({ errors: checked ? "1" : null })}
            />
            {ui("Errors only")}
          </label>
          <label className="flex items-center gap-2 text-xs">
            <Checkbox
              checked={showLifecycle}
              onCheckedChange={(checked) => setParams({ lifecycle: checked ? "1" : null })}
            />
            {ui("Lifecycle events")}
          </label>
          <TraceSearch sessionId={sessionId} onPick={(sequence) => void goToSequence(sequence)} />
        </div>
        {totals && (
          <div className="trace-totals ml-auto font-mono text-[11px] text-muted-foreground">
            {formatCount(totals.turn_count)} {ui("turns ·")} {formatCount(totals.step_count)}{" "}
            {ui("steps")}
            {totals.error_count > 0 && (
              <span className="text-destructive">
                {" "}
                · {formatCount(totals.error_count)} {ui("errors")}
              </span>
            )}
          </div>
        )}
      </div>

      <div className="trace-columns bg-brand-dots grid min-h-0 min-w-0 flex-1">
        {railVisible && (
          <aside className="min-h-0 overflow-y-auto" aria-label={ui("Turns")}>
            {rail}
          </aside>
        )}

        <div
          ref={scrollRef}
          data-trace-list="true"
          className="trace-list min-h-0 min-w-0 overflow-y-auto px-3 py-4"
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
              {ui("Could not load the trace:")} {data.error.message}
            </div>
          ) : rows.length === 0 ? (
            <div className="border bg-card p-6 text-sm text-muted-foreground">
              {ui("No turns yet. Turns appear here as the session runs.")}
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
                        reactions={turnReactions.get(row.turn.turn)?.prompt}
                        collapsed={row.collapsed}
                        inputSelected={turnInput === row.turn.turn}
                        onOpenInput={() => {
                          clearSelection();
                          setTurnInput(row.turn.turn);
                        }}
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
                        plainText={plainText}
                        selected={
                          !batch &&
                          !childStep &&
                          selectedStep?.turn === row.turn.turn &&
                          selectedStep.step === row.step.step
                        }
                        onSelect={() => selectStep(row.turn.turn, row.step.step)}
                        expand={
                          row.step.kind === "agent" && row.step.child_session_id
                            ? {
                                open: expansions.expanded.has(row.key),
                                label: expansions.expanded.has(row.key)
                                  ? "Hide the sub-agent's steps"
                                  : "Show the sub-agent's steps",
                                onToggle: () => void expansions.toggleAgent(row.step),
                              }
                            : undefined
                        }
                      />
                    )}
                    {row.type === "member" && (
                      <StepRow
                        turn={row.turn}
                        step={row.step}
                        view={view}
                        plainText={plainText}
                        nested
                        selected={
                          !batch &&
                          (row.childSessionId
                            ? childStep?.sessionId === row.childSessionId &&
                              childStep.turn === row.step.turn &&
                              childStep.step === row.step.step
                            : !childStep &&
                              selectedStep?.turn === row.step.turn &&
                              selectedStep.step === row.step.step)
                        }
                        onSelect={() => {
                          if (!row.childSessionId) {
                            selectStep(row.step.turn, row.step.step);
                            return;
                          }
                          setBatch(null);
                          setTurnInput(null);
                          setFull(false);
                          setChildStep({
                            sessionId: row.childSessionId,
                            turn: row.step.turn,
                            step: row.step.step,
                          });
                        }}
                      />
                    )}
                    {row.type === "more" && (
                      <ExpansionFooter
                        expansion={row.expansion}
                        onMore={() => {
                          const parent = rows.find((r) => r.key === row.parent);
                          if (parent?.type === "batch")
                            expansions.moreBatch(parent.turn.turn, parent.batch);
                        }}
                      />
                    )}
                    {row.type === "lifecycle" && <LifecycleRow turn={row.turn} event={row.event} />}
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
                          setTurnInput(null);
                          setChildStep(null);
                          setParams({ step: null });
                        }}
                        expand={{
                          open: expansions.expanded.has(row.key),
                          label: expansions.expanded.has(row.key) ? "Hide calls" : "Show calls",
                          onToggle: () => expansions.toggleBatch(row.turn.turn, row.batch),
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
                    {row.type === "footer" && (
                      <TurnFooter
                        turn={row.turn}
                        reactions={turnReactions.get(row.turn.turn)?.reply}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {wide && (
          <aside
            className="min-h-0 min-w-0 overflow-y-auto border-l bg-card"
            aria-label={turnInput ? ui("Turn input") : ui("Step inspector")}
          >
            {inspectorOpen && (
              <div className="flex justify-end border-b px-3 py-1">
                <Button variant="ghost" size="sm" onClick={clearSelection}>
                  {ui("Close")}
                </Button>
              </div>
            )}
            {inspector}
          </aside>
        )}
        {!wide && (
          <Drawer
            open={inspectorOpen && !request}
            onOpenChange={(open) => {
              if (!open && !request) clearSelection();
            }}
          >
            <DrawerContent
              className="!w-full !max-w-[min(100%,40rem)] gap-0 p-0"
              showCloseButton={false}
            >
              <div className="flex shrink-0 items-center justify-between gap-3 border-b px-4 py-2">
                <DrawerTitle className="text-sm">
                  {turnInput ? ui("Turn input") : ui("Step inspector")}
                </DrawerTitle>
                <Button variant="ghost" size="sm" onClick={clearSelection}>
                  {ui("Back to trace")}
                </Button>
              </div>
              <div className="min-h-0 min-w-0 flex-1 overflow-y-auto">{inspector}</div>
            </DrawerContent>
          </Drawer>
        )}
        {!railVisible && (
          <Drawer open={turnsOpen} onOpenChange={setTurnsOpen}>
            <DrawerContent side="left" className="!w-80 !max-w-full gap-0 p-0">
              <DrawerTitle className="border-b px-6 py-4 text-sm">{ui("Turns")}</DrawerTitle>
              <div className="min-h-0 overflow-y-auto pr-5">{rail}</div>
            </DrawerContent>
          </Drawer>
        )}
      </div>
      {/* Keep the request modal mounted when the inspector moves into a drawer. */}
      {request && (
        <RequestSheet
          {...request}
          open
          onOpenChange={(open) => {
            if (!open) setRequest(null);
          }}
        />
      )}
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
