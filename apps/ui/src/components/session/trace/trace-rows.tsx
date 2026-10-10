"use client";

// Row components of the Trace list: turn header and footer, step rows per
// kind, batches, gaps and the unloaded-turns bars. Layout follows the design
// handoff: gutter, spine with an icon tile, content, waterfall.

import Link from "next/link";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  ChevronsDown,
  ChevronsUp,
  CircleAlert,
  CircleDot,
  GitBranch,
  Layers,
  Send,
  ShieldCheck,
  Sparkles,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import type { TraceBatch, TraceGap, TraceStep, TraceTurn } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import {
  formatClock,
  formatCount,
  formatDuration,
  formatOffset,
  turnSpanMs,
  turnSummary,
  waterfallBar,
  type Expansion,
  type LifecycleEvent,
} from "./trace-model";

const KIND_ICON: Record<string, LucideIcon> = {
  model: Sparkles,
  answer: Bot,
  tool: Wrench,
  approval: ShieldCheck,
  agent: GitBranch,
  send: Send,
};

const BAR_COLOR: Record<string, string> = {
  model: "bg-primary/25",
  answer: "bg-primary/25",
  tool: "bg-accent",
  approval: "bg-success",
  agent: "bg-info/60",
  send: "bg-info/60",
};

export function UnloadedBar({
  edge,
  from,
  to,
  steps,
  errorTurns,
  busy,
  onLoad,
  onJumpToError,
}: {
  edge: "earlier" | "later";
  from: number;
  to: number;
  steps?: number;
  /** Turns with errors in the range. */
  errorTurns?: number;
  busy: boolean;
  onLoad: () => void;
  onJumpToError?: () => void;
}) {
  const Icon = edge === "earlier" ? ChevronsUp : ChevronsDown;
  return (
    <div className="flex flex-wrap items-center gap-3 border bg-muted/50 px-4 py-2.5 text-sm">
      <Icon className="size-4 text-muted-foreground" aria-hidden />
      <span className="font-medium">
        Turns {formatCount(from)}–{formatCount(to)} not loaded
      </span>
      <span className="text-muted-foreground">
        {[
          steps !== undefined ? `${formatCount(steps)} steps` : null,
          errorTurns
            ? `${formatCount(errorTurns)} turn${errorTurns === 1 ? "" : "s"} with errors`
            : null,
        ]
          .filter(Boolean)
          .join(" · ")}
      </span>
      <div className="ml-auto flex gap-2">
        {onJumpToError && (
          <Button variant="outline" size="sm" onClick={onJumpToError} disabled={busy}>
            Jump to error
          </Button>
        )}
        <Button variant="outline" size="sm" onClick={onLoad} disabled={busy}>
          {edge === "earlier" ? "Load 20 earlier" : "Load 20 later"}
        </Button>
      </div>
    </div>
  );
}

export function TurnHeader({
  turn,
  collapsed,
  onToggle,
  showScale,
}: {
  turn: TraceTurn;
  collapsed: boolean;
  onToggle: () => void;
  showScale: boolean;
}) {
  const Chevron = collapsed ? ChevronRight : ChevronDown;
  const duration = turn.duration_ms ?? null;
  return (
    <div className="mt-5 border border-b-0 bg-card first:mt-0">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!collapsed}
        className="flex w-full items-start gap-3 border-b bg-accent/9 px-4 py-3.5 text-left hover:bg-accent/14"
      >
        <Chevron className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
        <div className="min-w-0 flex-1">
          <div className="font-mono text-[11px] tracking-[0.04em] text-muted-foreground">
            TURN {formatCount(turn.turn)} · {formatClock(turn.started_at)} · user
          </div>
          <div className="mt-1 line-clamp-3 text-sm font-medium leading-normal break-words">
            {turn.prompt || <span className="italic text-muted-foreground">No prompt</span>}
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-x-3 gap-y-1 text-xs text-muted-foreground">
          {turn.status === "running" ? (
            <span className="font-mono text-foreground">running</span>
          ) : (
            duration !== null && (
              <span className="font-mono text-foreground">{formatDuration(duration)}</span>
            )
          )}
          <span>{turnSummary(turn)}</span>
          {turn.error_count > 0 && (
            <span className="inline-flex items-center gap-1 text-destructive">
              <CircleAlert className="size-3.5" aria-hidden />
              {formatCount(turn.error_count)} error{turn.error_count === 1 ? "" : "s"}
            </span>
          )}
        </div>
      </button>
      {!collapsed && showScale && (
        <div className="hidden justify-end px-0 pt-1 min-[1100px]:flex">
          <div className="flex w-40 justify-between pr-4 font-mono text-[10px] tracking-[0.08em] text-muted-foreground">
            <span>0s</span>
            <span>{formatDuration(turnSpanMs(turn))}</span>
          </div>
        </div>
      )}
    </div>
  );
}

export function TurnFooter({ turn }: { turn: TraceTurn }) {
  const tokens = turn.input_tokens + turn.output_tokens;
  const label =
    turn.status === "running"
      ? "Turn running"
      : turn.status === "completed"
        ? "Turn completed"
        : `Turn ${turn.status}`;
  return (
    <div className="border border-t-0 bg-card">
      <div
        className={cn(
          "flex flex-wrap gap-x-2 border-t border-border/60 py-2.5 pr-4 pl-[84px] text-xs text-muted-foreground",
          turn.status === "failed" && "text-destructive",
        )}
      >
        <span className="font-medium">{label}</span>
        <span>
          {formatCount(turn.step_count)} step{turn.step_count === 1 ? "" : "s"}
          {tokens ? ` · ${formatCount(tokens)} tokens` : ""}
          {turn.duration_ms != null ? ` · ${formatDuration(turn.duration_ms)}` : ""}
        </span>
        {turn.error && <span className="w-full truncate text-destructive">{turn.error}</span>}
      </div>
    </div>
  );
}

export function EmptyTurnRow() {
  return (
    <div className="border-x bg-card py-3 pl-[84px] text-sm italic text-muted-foreground">
      No steps match the current filter.
    </div>
  );
}

function Waterfall({
  offsetMs,
  durationMs,
  spanMs,
  color,
  running,
}: {
  offsetMs: number;
  durationMs: number | null | undefined;
  spanMs: number;
  color: string;
  running?: boolean;
}) {
  const { left, width } = waterfallBar(offsetMs, durationMs, spanMs);
  return (
    <div className="hidden w-40 shrink-0 pt-4 pr-4 min-[1100px]:block" aria-hidden>
      <div className="relative h-1.5 bg-muted">
        <div
          className={cn("absolute inset-y-0", color, running && "animate-tool-progress-active")}
          style={{ left: `${left * 100}%`, width: `max(2px, ${width * 100}%)` }}
        />
      </div>
    </div>
  );
}

function RowShell({
  label,
  offset,
  icon: Icon,
  tile,
  selected,
  error,
  onSelect,
  children,
  waterfall,
  nested,
  expand,
}: {
  label: string;
  offset: string;
  icon: LucideIcon;
  tile: string;
  selected: boolean;
  error: boolean;
  onSelect?: () => void;
  children: React.ReactNode;
  waterfall: React.ReactNode;
  /** Shown inline under a batch or a sub-agent step. */
  nested?: boolean;
  expand?: ExpandToggle;
}) {
  return (
    <div
      role={onSelect ? "button" : undefined}
      tabIndex={onSelect ? 0 : undefined}
      aria-pressed={onSelect ? selected : undefined}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (onSelect && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        "relative flex border-x bg-card",
        onSelect && "cursor-pointer hover:bg-muted/70",
        error && "bg-destructive/4",
        selected && "bg-primary/5",
        nested && "pl-6",
      )}
    >
      {selected && <span className="absolute inset-y-0 left-0 w-0.5 bg-primary" aria-hidden />}
      {nested && <span className="absolute inset-y-0 left-[22px] w-px bg-info/40" aria-hidden />}
      <div className="w-[52px] shrink-0 py-2.5 pl-3 font-mono text-[11px] leading-tight">
        <div>{label}</div>
        <div className="text-muted-foreground">{offset}</div>
      </div>
      <div className="relative w-8 shrink-0" aria-hidden>
        <span className="absolute inset-y-0 left-[15px] w-px bg-border" />
        <span
          className={cn(
            "absolute top-[9px] left-[4px] flex size-[22px] items-center justify-center border",
            tile,
            error && "border-destructive/40 bg-destructive/8 text-destructive",
          )}
        >
          <Icon className="size-3.5" />
        </span>
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5 py-2.5 pr-3 pl-2">{children}</div>
      {expand && (
        <button
          type="button"
          aria-expanded={expand.open}
          aria-label={expand.label}
          title={expand.label}
          onClick={(e) => {
            e.stopPropagation();
            expand.onToggle();
          }}
          className="my-2 mr-2 flex size-6 shrink-0 items-center justify-center border text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          {expand.open ? (
            <ChevronDown className="size-3.5" />
          ) : (
            <ChevronRight className="size-3.5" />
          )}
        </button>
      )}
      {waterfall}
    </div>
  );
}

export interface ExpandToggle {
  open: boolean;
  label: string;
  onToggle: () => void;
}

function tileFor(kind: string): string {
  switch (kind) {
    case "tool":
      return "border-border bg-accent/12";
    case "agent":
    case "send":
      return "border-border bg-info/10";
    case "answer":
      return "border-primary bg-card";
    default:
      return "border-border bg-card";
  }
}

export function StepRow({
  turn,
  step,
  view,
  selected,
  onSelect,
  nested,
  expand,
}: {
  turn: TraceTurn;
  step: TraceStep;
  view: "all" | "messages" | "tools";
  selected: boolean;
  onSelect: () => void;
  nested?: boolean;
  expand?: ExpandToggle;
}) {
  const error = step.status === "error";
  const running = step.status === "running";
  const Icon = error ? CircleAlert : (KIND_ICON[step.kind] ?? Wrench);
  return (
    <RowShell
      label={`#${step.step}`}
      offset={formatOffset(step.offset_ms)}
      icon={Icon}
      tile={tileFor(step.kind)}
      selected={selected}
      error={error}
      onSelect={onSelect}
      nested={nested}
      expand={expand}
      waterfall={
        <Waterfall
          offsetMs={step.offset_ms}
          durationMs={running ? Date.now() - Date.parse(step.started_at) : step.duration_ms}
          spanMs={turnSpanMs(turn)}
          color={error ? "bg-destructive" : (BAR_COLOR[step.kind] ?? "bg-muted-foreground/30")}
          running={running}
        />
      }
    >
      <StepContent step={step} view={view} />
    </RowShell>
  );
}

function StepContent({ step, view }: { step: TraceStep; view: "all" | "messages" | "tools" }) {
  const error = step.status === "error";
  switch (step.kind) {
    case "model":
    case "answer": {
      const tokens =
        step.input_tokens != null || step.output_tokens != null
          ? `${formatCount(step.input_tokens ?? 0)} → ${formatCount(step.output_tokens ?? 0)} tok`
          : null;
      const requested = step.requested_tool_call_ids?.length ?? 0;
      return (
        <>
          {view === "all" && (
            <div className="flex flex-wrap gap-x-2 text-xs text-muted-foreground">
              <span className="font-medium text-foreground">
                {step.kind === "answer" ? "Final answer" : "Model call"}
              </span>
              {tokens && <span className="font-mono">{tokens}</span>}
              {step.duration_ms != null && (
                <span className="font-mono">{formatDuration(step.duration_ms)}</span>
              )}
              {requested > 0 && (
                <span>
                  requested {requested} tool call{requested === 1 ? "" : "s"}
                </span>
              )}
              {error && step.result && <span className="text-destructive">{step.result}</span>}
            </div>
          )}
          {step.narration && (
            <p
              className={cn(
                "text-sm leading-[1.55] break-words whitespace-pre-wrap",
                step.kind === "answer" && "border-l border-primary/65 pl-3.5",
              )}
            >
              {step.narration}
            </p>
          )}
        </>
      );
    }
    case "agent":
      return (
        <>
          <div className="flex min-w-0 items-center gap-2">
            <span className="text-xs font-medium text-muted-foreground">Sub-agent</span>
            {step.target && (
              <span className="truncate text-[13px] font-semibold">{step.target}</span>
            )}
            {step.child_session_id && (
              <Link
                href={`/sessions/${step.child_session_id}/trace`}
                onClick={(e) => e.stopPropagation()}
                className="border border-primary/30 bg-primary/6 px-1.5 font-mono text-[11px] hover:underline"
              >
                {step.child_session_id}
              </Link>
            )}
            <span className="ml-auto font-mono text-xs text-muted-foreground">
              {formatDuration(step.duration_ms)}
            </span>
          </div>
          {step.result && (
            <ResultLine error={error}>
              <span className="break-words">{step.result}</span>
            </ResultLine>
          )}
        </>
      );
    case "approval":
      return (
        <>
          <div className="text-[13px] font-medium">Approval recorded</div>
          <div className="flex flex-wrap gap-x-2 text-xs">
            {step.target && <span className="font-mono text-muted-foreground">{step.target}</span>}
            {step.result && (
              <span className={error ? "text-destructive" : "text-success"}>→ {step.result}</span>
            )}
          </div>
        </>
      );
    default:
      return (
        <>
          <div className="flex min-w-0 items-baseline gap-2">
            <span className="shrink-0 font-mono text-[13px] font-semibold whitespace-nowrap">
              {step.name ?? step.kind}
            </span>
            {step.target && (
              <span className="min-w-0 truncate font-mono text-xs text-muted-foreground">
                {step.target}
              </span>
            )}
            <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground">
              {step.status === "running" ? "running" : formatDuration(step.duration_ms)}
            </span>
          </div>
          {step.result && (
            <ResultLine error={error}>
              <span className="truncate">{step.result}</span>
            </ResultLine>
          )}
        </>
      );
  }
}

function ResultLine({ error, children }: { error: boolean; children: React.ReactNode }) {
  return (
    <div
      className={cn(
        "flex min-w-0 gap-1 text-xs",
        error ? "text-destructive" : "text-muted-foreground",
      )}
    >
      <span aria-hidden>→</span>
      {children}
    </div>
  );
}

export function BatchRow({
  turn,
  batch,
  selected,
  onSelect,
  expand,
}: {
  turn: TraceTurn;
  batch: TraceBatch;
  selected: boolean;
  onSelect: () => void;
  expand?: ExpandToggle;
}) {
  const okShare = batch.count ? batch.succeeded / batch.count : 0;
  return (
    <RowShell
      label={`#${batch.first_step}`}
      offset={formatOffset(batch.offset_ms)}
      icon={Layers}
      tile="border-border bg-accent/12"
      selected={selected}
      error={batch.failed > 0 && batch.succeeded === 0}
      onSelect={onSelect}
      expand={expand}
      waterfall={
        <Waterfall
          offsetMs={batch.offset_ms}
          durationMs={batch.wall_ms}
          spanMs={turnSpanMs(turn)}
          color={batch.failed > 0 && batch.succeeded === 0 ? "bg-destructive" : "bg-accent"}
          running={batch.running > 0}
        />
      }
    >
      <div className="flex min-w-0 items-baseline gap-2">
        <span className="font-mono text-[13px] font-semibold">{batch.name}</span>
        <span className="border bg-muted px-1 font-mono text-[11px] text-muted-foreground">
          ×{formatCount(batch.count)}
        </span>
        <span className="ml-auto font-mono text-xs text-muted-foreground">
          {formatDuration(batch.wall_ms)}
        </span>
      </div>
      <div className="flex h-1 gap-px" aria-hidden>
        {batch.succeeded > 0 && <div className="bg-accent" style={{ flexGrow: okShare }} />}
        {batch.failed > 0 && (
          <div className="min-w-[3px] bg-destructive" style={{ flexGrow: 1 - okShare }} />
        )}
      </div>
      <div className="flex flex-wrap gap-x-2 text-xs text-muted-foreground">
        <span>
          {formatCount(batch.succeeded)} succeeded
          {batch.p50_ms != null ? ` · p50 ${formatDuration(batch.p50_ms)}` : ""}
          {batch.p95_ms != null ? ` · p95 ${formatDuration(batch.p95_ms)}` : ""}
        </span>
        {batch.failed > 0 && (
          <span className="text-destructive">{formatCount(batch.failed)} failed</span>
        )}
        {batch.running > 0 && <span>{formatCount(batch.running)} running</span>}
      </div>
    </RowShell>
  );
}

export function GapRow({
  gap,
  busy,
  onShowMore,
}: {
  gap: TraceGap;
  busy: boolean;
  onShowMore: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3 border-x bg-muted/40 py-2.5 pr-4 pl-[84px] text-xs text-muted-foreground">
      <span>
        {formatCount(gap.count)} steps hidden (#{gap.first_step}–#{gap.last_step})
      </span>
      {gap.errors > 0 && (
        <span className="text-destructive">
          {formatCount(gap.errors)} error{gap.errors === 1 ? "" : "s"}
        </span>
      )}
      <Button
        variant="link"
        size="sm"
        className="h-auto p-0 text-xs"
        onClick={onShowMore}
        disabled={busy}
      >
        Show {formatCount(Math.min(100, gap.count))} more
      </Button>
    </div>
  );
}

/** Status line under an expanded batch or sub-agent: loading, more, errors. */
export function ExpansionFooter({
  expansion,
  onMore,
}: {
  expansion: Expansion;
  onMore?: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3 border-x bg-muted/40 py-2 pr-4 pl-[108px] text-xs text-muted-foreground">
      {expansion.loading ? (
        <span>Loading…</span>
      ) : expansion.error ? (
        <span className="text-destructive">Could not load: {expansion.error}</span>
      ) : expansion.steps.length === 0 ? (
        <span>No steps yet.</span>
      ) : null}
      {!expansion.loading && expansion.nextStep != null && onMore && (
        <Button variant="link" size="sm" className="h-auto p-0 text-xs" onClick={onMore}>
          Show more calls from #{expansion.nextStep}
        </Button>
      )}
      {!expansion.loading && expansion.childSessionId && (
        <>
          {!!expansion.hiddenTurns && (
            <span>
              {formatCount(expansion.hiddenTurns)} earlier turn
              {expansion.hiddenTurns === 1 ? "" : "s"} not shown
            </span>
          )}
          <Link
            href={`/sessions/${expansion.childSessionId}/trace`}
            className="font-medium text-primary hover:underline"
          >
            Open the sub-agent&apos;s trace
          </Link>
        </>
      )}
    </div>
  );
}

/** An event no step shows (file writes, capability usage), with repeats folded. */
export function LifecycleRow({ turn, event }: { turn: TraceTurn; event: LifecycleEvent }) {
  const offset = Math.max(0, Date.parse(event.ts) - Date.parse(turn.started_at));
  const failed = /fail|error/.test(event.type);
  return (
    <div className="flex items-center border-x bg-card text-xs">
      <div className="w-[52px] shrink-0 py-1.5 pl-3 font-mono text-[11px] text-muted-foreground">
        {formatOffset(offset)}
      </div>
      <div className="relative w-8 shrink-0 self-stretch" aria-hidden>
        <span className="absolute inset-y-0 left-[15px] w-px bg-border" />
        <CircleDot className="absolute top-1/2 left-[10px] size-[11px] -translate-y-1/2 bg-card text-muted-foreground" />
      </div>
      <span
        className={cn(
          "min-w-0 truncate py-1.5 pl-2 font-mono",
          failed ? "text-destructive" : "text-muted-foreground",
        )}
      >
        {event.type}
      </span>
      {event.count > 1 && (
        <span className="ml-2 border bg-muted px-1 font-mono text-[11px] text-muted-foreground">
          ×{formatCount(event.count)}
        </span>
      )}
      <span className="ml-auto pr-4 font-mono text-[11px] text-muted-foreground">
        #{event.sequence}
      </span>
    </div>
  );
}
