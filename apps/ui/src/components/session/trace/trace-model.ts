// Pure model of the session Trace tab: which rows show for the loaded turns,
// view filters, and formatting. Kept free of React so it is unit-tested.
//
// Decision: the list is one flat array of rows (turn header, steps, batches,
// gaps, turn footer) so a single virtualizer renders any number of loaded
// turns with variable row heights. See knowledge/ui/session-trace.md.

import type {
  TraceBatch,
  TraceEventRef,
  TraceGap,
  TraceItem,
  TraceStep,
  TraceTurn,
} from "@/lib/api/types";

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
  | { type: "footer"; key: string; turn: TraceTurn }
  | {
      type: "member";
      key: string;
      turn: TraceTurn;
      parent: string;
      step: TraceStep;
      /** Set when the step belongs to a sub-agent's session. */
      childSessionId?: string;
    }
  | { type: "more"; key: string; turn: TraceTurn; parent: string; expansion: Expansion }
  | { type: "lifecycle"; key: string; turn: TraceTurn; event: LifecycleEvent };

/**
 * Steps shown inline under a batch (its calls) or a sub-agent step (the
 * sub-agent's own steps, one level deep). Keyed by the parent row's key.
 */
export interface Expansion {
  steps: TraceStep[];
  /** Step to continue from, when the batch has more calls. */
  nextStep?: number | null;
  loading: boolean;
  error?: string;
  /** Session the steps come from, when it is a sub-agent's. */
  childSessionId?: string;
  /** Turns of the sub-agent beyond the one shown. */
  hiddenTurns?: number;
}

/** A run of identical lifecycle events, shown as one row. */
export interface LifecycleEvent {
  type: string;
  sequence: number;
  ts: string;
  count: number;
}

export interface TraceExtras {
  expanded?: ReadonlyMap<string, Expansion>;
  /** Lifecycle events per turn number, when the lifecycle toggle is on. */
  lifecycle?: ReadonlyMap<number, LifecycleEvent[]>;
}

/** Row key of a batch; also the key of its expansion. */
export function batchKey(turn: number, firstStep: number): string {
  return `b${turn}.${firstStep}`;
}

/** Row key of a step; also the key of a sub-agent step's expansion. */
export function stepRowKey(turn: number, step: number): string {
  return `s${turn}.${step}`;
}

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

// Event types the step rows already show; the lifecycle toggle adds the rest.
const STEP_EVENT_PREFIXES = [
  "turn.",
  "llm.",
  "tool.",
  "input.",
  "output.message.",
  "reason.started",
  "reason.completed",
  "act.",
];

function isStepEvent(type: string): boolean {
  return STEP_EVENT_PREFIXES.some((prefix) => type.startsWith(prefix));
}

/** Lifecycle events of a turn: those no step shows, with repeats collapsed. */
export function lifecycleEvents(events: TraceEventRef[]): LifecycleEvent[] {
  const out: LifecycleEvent[] = [];
  for (const event of events) {
    if (isStepEvent(event.type)) continue;
    const last = out[out.length - 1];
    if (last && last.type === event.type) {
      last.count += 1;
      continue;
    }
    out.push({ type: event.type, sequence: event.sequence, ts: event.ts, count: 1 });
  }
  return out;
}

function lifecycleVisible(event: LifecycleEvent, filters: TraceFilters): boolean {
  return !filters.errorsOnly || /fail|error/.test(event.type);
}

function pushExpansion(
  rows: TraceRow[],
  turn: TraceTurn,
  parent: string,
  expansion: Expansion | undefined,
  filters: TraceFilters,
) {
  if (!expansion) return;
  for (const step of expansion.steps) {
    // A batch's calls follow the error filter; a sub-agent's steps all show.
    if (!expansion.childSessionId && filters.errorsOnly && step.status !== "error") continue;
    rows.push({
      type: "member",
      key: `m${parent}:${step.turn}.${step.step}`,
      turn,
      parent,
      step,
      childSessionId: expansion.childSessionId,
    });
  }
  if (
    expansion.loading ||
    expansion.error ||
    expansion.nextStep != null ||
    expansion.hiddenTurns ||
    expansion.steps.length === 0
  ) {
    rows.push({ type: "more", key: `x${parent}`, turn, parent, expansion });
  }
}

/** Flatten loaded turns into list rows. `turns` must be in turn order. */
export function buildRows(
  turns: TraceTurn[],
  turnCount: number,
  filters: TraceFilters,
  collapsed: ReadonlySet<number>,
  extras: TraceExtras = {},
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
    const lifecycle = (extras.lifecycle?.get(turn.turn) ?? []).filter((e) =>
      lifecycleVisible(e, filters),
    );
    let next = 0;
    // Lifecycle rows go before the first step that started after them.
    const flushBefore = (sequence: number) => {
      while (next < lifecycle.length && lifecycle[next]!.sequence < sequence) {
        const event = lifecycle[next]!;
        rows.push({ type: "lifecycle", key: `l${turn.turn}.${event.sequence}`, turn, event });
        next += 1;
      }
    };
    let shown = 0;
    for (const item of turn.items) {
      if (item.type === "step") flushBefore(item.start_sequence);
      if (!itemVisible(item, filters)) continue;
      shown += 1;
      switch (item.type) {
        case "step": {
          const key = stepRowKey(turn.turn, item.step);
          rows.push({ type: "step", key, turn, step: item });
          pushExpansion(rows, turn, key, extras.expanded?.get(key), filters);
          break;
        }
        case "batch": {
          const key = batchKey(turn.turn, item.first_step);
          rows.push({ type: "batch", key, turn, batch: item });
          pushExpansion(rows, turn, key, extras.expanded?.get(key), filters);
          break;
        }
        case "gap":
          rows.push({ type: "gap", key: `g${turn.turn}.${item.first_step}`, turn, gap: item });
          break;
      }
    }
    flushBefore(Number.POSITIVE_INFINITY);
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

/**
 * Steps in the order the inspector's previous/next and j/k walk them: this
 * session's steps, including a batch's expanded calls. A sub-agent's steps are
 * selected by clicking only, since their numbers belong to another session.
 */
export function selectableKeys(rows: TraceRow[]): string[] {
  const keys: string[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    if (row.type !== "step" && !(row.type === "member" && !row.childSessionId)) continue;
    const key = stepKey(row.step.turn, row.step.step);
    if (seen.has(key)) continue;
    seen.add(key);
    keys.push(key);
  }
  return keys;
}

/** The row showing a step of this session: its own row, or a batch member. */
export function rowIndexOfStep(rows: TraceRow[], turn: number, step: number): number {
  return rows.findIndex(
    (r) =>
      (r.type === "step" || (r.type === "member" && !r.childSessionId)) &&
      r.step.turn === turn &&
      r.step.step === step,
  );
}

/** The turn of a page that holds an event sequence, or the nearest before it. */
export function turnHoldingSequence(turns: TraceTurn[], sequence: number): TraceTurn | undefined {
  let found: TraceTurn | undefined;
  for (const turn of turns) {
    if (turn.start_sequence <= sequence) found = turn;
  }
  return found ?? turns[0];
}

/**
 * The loaded step of a turn that holds a sequence: the step whose events
 * include it, else the last step that started before it (an event such as
 * `output.message.completed` lands just after its model call ends).
 */
export function stepHoldingSequence(turn: TraceTurn, sequence: number): TraceStep | undefined {
  let latest: TraceStep | undefined;
  for (const item of turn.items) {
    if (item.type !== "step" || item.start_sequence > sequence) continue;
    if (item.end_sequence != null && item.end_sequence >= sequence) return item;
    latest = item;
  }
  return latest;
}

/**
 * A short excerpt of a search hit around the first word of the query, from an
 * event's payload.
 */
export function searchSnippet(data: unknown, query: string, width = 90): string {
  const text = (typeof data === "string" ? data : JSON.stringify(data ?? ""))
    .replace(/\\[nt]/g, " ")
    .replace(/\s+/g, " ");
  const word = query.trim().split(/\s+/)[0]?.toLowerCase() ?? "";
  const at = word ? text.toLowerCase().indexOf(word) : -1;
  if (at === -1) return text.slice(0, width);
  const start = Math.max(0, at - Math.floor(width / 3));
  const end = Math.min(text.length, start + width);
  return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
}
