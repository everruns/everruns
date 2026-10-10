// Pure model of the session Trace tab: which rows show for the loaded turns,
// view filters, and formatting. Kept free of React so it is unit-tested.
//
// Decision: the list is one flat array of rows (turn header, steps, batches,
// gaps, turn footer) so a single virtualizer renders any number of loaded
// turns with variable row heights. See knowledge/ui/session-trace.md.

import type { TraceBatch, TraceGap, TraceItem, TraceStep, TraceTurn } from "@/lib/api/types";

export type TraceView = "all" | "messages" | "tools";

export interface TraceFilters {
  view: TraceView;
  errorsOnly: boolean;
}

export type TraceRow =
  | { type: "unloaded"; key: string; edge: "earlier" | "later"; from: number; to: number }
  | { type: "turn"; key: string; turn: TraceTurn; collapsed: boolean }
  | { type: "step"; key: string; turn: TraceTurn; step: TraceStep }
  | { type: "batch"; key: string; turn: TraceTurn; batch: TraceBatch }
  | { type: "gap"; key: string; turn: TraceTurn; gap: TraceGap }
  | { type: "empty"; key: string; turn: TraceTurn }
  | { type: "footer"; key: string; turn: TraceTurn };

/** Stable id of a step, as used in the URL: `<turn>.<step>`. */
export function stepKey(turn: number, step: number): string {
  return `${turn}.${step}`;
}

export function parseStepKey(
  value: string | null | undefined,
): { turn: number; step: number } | null {
  if (!value) return null;
  const match = /^(\d+)\.(\d+)$/.exec(value);
  if (!match) return null;
  return { turn: Number(match[1]), step: Number(match[2]) };
}

const MESSAGE_KINDS = new Set(["answer", "agent", "send"]);
const TOOL_KINDS = new Set(["tool", "approval", "agent", "send"]);

/** Whether a step shows under the current view and error filter. */
export function stepVisible(step: TraceStep, filters: TraceFilters): boolean {
  if (filters.errorsOnly && step.status !== "error" && step.kind !== "answer") return false;
  switch (filters.view) {
    case "messages":
      // The message stream: narration, answers, sub-agents and messages.
      return MESSAGE_KINDS.has(step.kind) || (step.kind === "model" && !!step.narration);
    case "tools":
      return TOOL_KINDS.has(step.kind);
    default:
      return true;
  }
}

function itemVisible(item: TraceItem, filters: TraceFilters): boolean {
  switch (item.type) {
    case "step":
      return stepVisible(item, filters);
    case "batch":
      return filters.view !== "messages" && (!filters.errorsOnly || item.failed > 0);
    case "gap":
      return !filters.errorsOnly || item.errors > 0;
  }
}

/** Flatten loaded turns into list rows. `turns` must be in turn order. */
export function buildRows(
  turns: TraceTurn[],
  turnCount: number,
  filters: TraceFilters,
  collapsed: ReadonlySet<number>,
): TraceRow[] {
  const rows: TraceRow[] = [];
  const first = turns[0]?.turn;
  const last = turns[turns.length - 1]?.turn;
  if (first !== undefined && first > 1) {
    rows.push({ type: "unloaded", key: "earlier", edge: "earlier", from: 1, to: first - 1 });
  }
  for (const turn of turns) {
    const isCollapsed = collapsed.has(turn.turn);
    rows.push({ type: "turn", key: `t${turn.turn}`, turn, collapsed: isCollapsed });
    if (isCollapsed) continue;
    let shown = 0;
    for (const item of turn.items) {
      if (!itemVisible(item, filters)) continue;
      shown += 1;
      switch (item.type) {
        case "step":
          rows.push({ type: "step", key: `s${stepKey(turn.turn, item.step)}`, turn, step: item });
          break;
        case "batch":
          rows.push({ type: "batch", key: `b${turn.turn}.${item.first_step}`, turn, batch: item });
          break;
        case "gap":
          rows.push({ type: "gap", key: `g${turn.turn}.${item.first_step}`, turn, gap: item });
          break;
      }
    }
    if (shown === 0 && turn.items.length > 0) {
      rows.push({ type: "empty", key: `e${turn.turn}`, turn });
    }
    rows.push({ type: "footer", key: `f${turn.turn}`, turn });
  }
  if (last !== undefined && last < turnCount) {
    rows.push({ type: "unloaded", key: "later", edge: "later", from: last + 1, to: turnCount });
  }
  return rows;
}

/** Merge a page into the loaded turns, replacing turns it repeats. */
export function mergeTurns(loaded: TraceTurn[], page: TraceTurn[]): TraceTurn[] {
  const byTurn = new Map(loaded.map((t) => [t.turn, t]));
  for (const turn of page) byTurn.set(turn.turn, turn);
  return [...byTurn.values()].sort((a, b) => a.turn - b.turn);
}

/** Replace a gap with the steps fetched for its start, keeping the rest as a gap. */
export function fillGap(
  turn: TraceTurn,
  gap: TraceGap,
  steps: TraceStep[],
  nextStep?: number | null,
): TraceTurn {
  const items: TraceItem[] = [];
  for (const item of turn.items) {
    if (item.type !== "gap" || item.first_step !== gap.first_step) {
      items.push(item);
      continue;
    }
    for (const step of steps) items.push({ ...step, type: "step" });
    if (nextStep != null && nextStep <= gap.last_step) {
      const errors = steps.filter((s) => s.status === "error").length;
      items.push({
        type: "gap",
        turn: gap.turn,
        first_step: nextStep,
        last_step: gap.last_step,
        count: Math.max(0, gap.count - steps.length),
        errors: Math.max(0, gap.errors - errors),
      });
    }
  }
  return { ...turn, items };
}

/** Turn duration for the waterfall scale; a running turn runs until `now`. */
export function turnSpanMs(turn: TraceTurn, now: number = Date.now()): number {
  if (turn.duration_ms != null) return Math.max(1, turn.duration_ms);
  return Math.max(1, now - Date.parse(turn.started_at));
}

/** Left offset and width of a waterfall bar, as fractions of the turn. */
export function waterfallBar(
  offsetMs: number,
  durationMs: number | null | undefined,
  spanMs: number,
): { left: number; width: number } {
  const left = Math.min(1, Math.max(0, offsetMs / spanMs));
  const width = Math.min(1 - left, Math.max(0, (durationMs ?? 0) / spanMs));
  return { left, width };
}

export function formatDuration(ms: number | null | undefined): string {
  if (ms == null) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  if (minutes < 60) return rest ? `${minutes}m ${rest}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export function formatOffset(ms: number): string {
  return `+${formatDuration(ms) || "0ms"}`;
}

export function formatCount(value: number): string {
  return value.toLocaleString("en-US");
}

export function formatTokens(value: number | null | undefined): string {
  if (value == null) return "";
  if (value < 1000) return String(value);
  if (value < 10_000) return `${(value / 1000).toFixed(1)}K`;
  if (value < 1_000_000) return `${Math.round(value / 1000)}K`;
  return `${(value / 1_000_000).toFixed(1)}M`;
}

export function formatClock(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** "3 model · 5 tool · 2 sub-agents" for a turn header. */
export function turnSummary(turn: TraceTurn): string {
  const parts: string[] = [];
  if (turn.model_calls) parts.push(`${formatCount(turn.model_calls)} model`);
  if (turn.tool_calls) parts.push(`${formatCount(turn.tool_calls)} tool`);
  if (turn.subagent_calls) {
    parts.push(
      `${formatCount(turn.subagent_calls)} sub-agent${turn.subagent_calls === 1 ? "" : "s"}`,
    );
  }
  return parts.join(" · ");
}

/** Which minimap bucket holds a turn. */
export function bucketIndex(turn: number, bucketSize: number): number {
  return Math.floor((turn - 1) / Math.max(1, bucketSize));
}

/** Activity of a bucket on 0..1, from its share of the busiest bucket. */
export function bucketIntensity(steps: number, maxSteps: number): number {
  if (maxSteps <= 0) return 0;
  return Math.min(1, steps / maxSteps);
}

/** Steps in the order the inspector's previous/next walk them. */
export function selectableKeys(rows: TraceRow[]): string[] {
  const keys: string[] = [];
  for (const row of rows) {
    if (row.type === "step") keys.push(stepKey(row.turn.turn, row.step.step));
  }
  return keys;
}
