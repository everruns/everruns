"use client";

// The run as it happened (EVE-854): a chronological, read-only rail of the
// structural steps behind a session — turn boundaries with the model and prompt
// tokens, tool calls with their result payload inline, subagent tasks with what
// they returned, failures/retries with the durable execution that ran them, and
// session close.
//
// The transcript answers "what was said"; this answers "what ran". Both are
// derived purely from the event stream, so a live session streams into it and a
// finished one replays from history with no live connection.

import { useMemo, useState } from "react";
import {
  AlertTriangle,
  Bot,
  ChevronDown,
  ChevronRight,
  CircleDot,
  Flag,
  ListTodo,
  Play,
  Sparkles,
  Wrench,
} from "lucide-react";
import type { ComponentType } from "react";
import type { Event, SessionTask } from "@/lib/api/types";
import { getEventData, getTextFromContent } from "@/lib/api/types";
import { formatTokens } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";

type StepTone = "neutral" | "accent" | "danger";

interface TimelineStep {
  key: string;
  /** Sort key: event sequence when available, else the timestamp. */
  order: number;
  ts: string;
  icon: ComponentType<{ className?: string }>;
  tone: StepTone;
  title: string;
  /** Short user-facing facts rendered as a badge row (model, tokens, duration). */
  facts: string[];
  /** Raw correlation and storage fields hidden behind the details disclosure. */
  technicalDetails?: string[];
  /** Expandable payload — a tool result, a subagent return, an error. */
  payload?: string;
}

const TONE_CLASSNAMES: Record<StepTone, string> = {
  neutral: "text-muted-foreground",
  accent: "text-primary",
  danger: "text-destructive",
};

function formatDuration(ms: number | undefined): string | null {
  if (ms == null) return null;
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`;
}

/** Render a tool result / subagent return as text, falling back to JSON. */
function stringifyPayload(value: unknown): string | undefined {
  if (value == null) return undefined;
  if (typeof value === "string") return value.trim() || undefined;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return undefined;
  }
}

/**
 * The durable-execution worker that ran a step. Events carry it as the
 * correlation `exec_id`; when the runtime does not attach one, we say nothing
 * rather than inventing a name.
 */
function workerDetails(event: Event): string[] {
  const execId = event.context?.exec_id;
  return execId ? [`Worker: ${execId}`] : [];
}

function eventSteps(events: Event[]): TimelineStep[] {
  const steps: TimelineStep[] = [];

  events.forEach((event, index) => {
    const order = event.sequence ?? index;
    const base = { key: event.id, order, ts: event.ts };

    const started = getEventData(event, "session.started");
    if (started) {
      steps.push({
        ...base,
        icon: Play,
        tone: "accent",
        title: "Session started",
        facts: [started.model_id ? `model ${started.model_id}` : null].filter(
          (fact): fact is string => !!fact,
        ),
        technicalDetails: workerDetails(event),
      });
      return;
    }

    const turnStarted = getEventData(event, "turn.started");
    if (turnStarted) {
      steps.push({
        ...base,
        icon: CircleDot,
        tone: "accent",
        title: "Agent turn started",
        facts: [],
        technicalDetails: workerDetails(event),
        payload: stringifyPayload(turnStarted.input_content),
      });
      return;
    }

    const generation = getEventData(event, "llm.generation");
    if (generation) {
      const usage = generation.metadata.usage;
      steps.push({
        ...base,
        icon: Sparkles,
        tone: generation.metadata.success ? "neutral" : "danger",
        title: "Model call",
        facts: [
          generation.metadata.model,
          usage ? `${formatTokens(usage.input_tokens)} prompt` : null,
          usage ? `${formatTokens(usage.output_tokens)} completion` : null,
          formatDuration(generation.metadata.duration_ms),
          generation.metadata.retry && generation.metadata.retry.attempts > 0
            ? `${generation.metadata.retry.attempts + 1} attempts`
            : null,
        ].filter((fact): fact is string => !!fact),
        technicalDetails: workerDetails(event),
        payload: generation.metadata.error,
      });
      return;
    }

    const toolCompleted = getEventData(event, "tool.completed");
    if (toolCompleted) {
      const resultText = toolCompleted.result ? getTextFromContent(toolCompleted.result) : "";
      steps.push({
        ...base,
        icon: Wrench,
        tone: toolCompleted.success ? "neutral" : "danger",
        title: toolCompleted.display_name || toolCompleted.tool_name,
        facts: [toolCompleted.status, formatDuration(toolCompleted.duration_ms)].filter(
          (fact): fact is string => !!fact,
        ),
        technicalDetails: workerDetails(event),
        payload:
          toolCompleted.error ??
          stringifyPayload(resultText || (toolCompleted.result as unknown)) ??
          undefined,
      });
      return;
    }

    const turnFailed = getEventData(event, "turn.failed");
    if (turnFailed) {
      steps.push({
        ...base,
        icon: AlertTriangle,
        tone: "danger",
        title: "Turn failed",
        facts: [turnFailed.error_code ?? null].filter((fact): fact is string => !!fact),
        technicalDetails: workerDetails(event),
        payload: turnFailed.error,
      });
      return;
    }

    const turnCompleted = getEventData(event, "turn.completed");
    if (turnCompleted) {
      steps.push({
        ...base,
        icon: CircleDot,
        tone: "neutral",
        title: "Agent turn completed",
        facts: [
          `${turnCompleted.iterations} ${turnCompleted.iterations === 1 ? "iteration" : "iterations"}`,
          turnCompleted.tool_call_count != null ? `${turnCompleted.tool_call_count} tools` : null,
          turnCompleted.usage
            ? `${formatTokens(turnCompleted.usage.input_tokens + turnCompleted.usage.output_tokens)} tokens`
            : null,
          formatDuration(turnCompleted.duration_ms),
        ].filter((fact): fact is string => !!fact),
        technicalDetails: workerDetails(event),
      });
      return;
    }

    const idled = getEventData(event, "session.idled");
    if (idled) {
      steps.push({
        ...base,
        icon: Flag,
        tone: "neutral",
        title: "Session closed the turn",
        facts: [
          idled.usage
            ? `${formatTokens(idled.usage.input_tokens + idled.usage.output_tokens)} cumulative tokens`
            : null,
        ].filter((fact): fact is string => !!fact),
        technicalDetails: workerDetails(event),
      });
    }
  });

  return steps;
}

/**
 * Subagents and background tasks are not events; they are task records with
 * their own lifecycle. Place each one at its creation time so "what the
 * subagent returned" reads in line with the run that spawned it.
 */
function taskSteps(tasks: SessionTask[], events: Event[]): TimelineStep[] {
  if (tasks.length === 0) return [];

  // Interleave by timestamp: find the sequence of the last event at or before
  // the task's creation, so tasks land between the right pair of events.
  const ordered = events
    .map((event, index) => ({ ts: event.ts, order: event.sequence ?? index }))
    .sort((a, b) => a.ts.localeCompare(b.ts));

  const orderFor = (ts: string): number => {
    let order = -1;
    for (const entry of ordered) {
      if (entry.ts > ts) break;
      order = entry.order;
    }
    return order + 0.5;
  };

  return tasks.map((task) => ({
    key: `task:${task.id}`,
    order: orderFor(task.created_at),
    ts: task.created_at,
    icon: task.kind === "subagent" ? Bot : ListTodo,
    tone: task.state === "failed" ? ("danger" as const) : ("neutral" as const),
    title: task.display_name || task.kind,
    facts: [task.kind, task.state, task.attempt > 1 ? `${task.attempt} attempts` : null].filter(
      (fact): fact is string => !!fact,
    ),
    technicalDetails: [
      task.worker_id ? `Worker: ${task.worker_id}` : null,
      task.result_path ? `Result path: ${task.result_path}` : null,
    ].filter((detail): detail is string => !!detail),
    payload: task.error ? `${task.error.kind}: ${task.error.message}` : (task.summary ?? undefined),
  }));
}

function TimelineRow({ step }: { step: TimelineStep }) {
  const [expanded, setExpanded] = useState(false);
  const Icon = step.icon;

  return (
    <li className="relative pl-8">
      <span
        className={cn(
          "absolute left-0 top-1 flex h-5 w-5 items-center justify-center rounded-full border border-border bg-card",
          TONE_CLASSNAMES[step.tone],
        )}
      >
        <Icon className="h-3 w-3" />
      </span>

      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
        <span className="text-sm font-medium text-foreground">{step.title}</span>
        <time className="text-[11px] tabular-nums text-muted-foreground" dateTime={step.ts}>
          {new Date(step.ts).toLocaleTimeString()}
        </time>
      </div>

      {step.facts.length > 0 && (
        <div className="mt-1 flex flex-wrap gap-1">
          {step.facts.map((fact) => (
            <Badge key={fact} variant="outline" className="font-normal">
              {fact}
            </Badge>
          ))}
        </div>
      )}

      {(step.payload || (step.technicalDetails?.length ?? 0) > 0) && (
        <div className="mt-1.5">
          <button
            type="button"
            onClick={() => setExpanded((value) => !value)}
            className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            aria-expanded={expanded}
          >
            {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
            {expanded ? "Hide details" : "Show details"}
          </button>
          {expanded && (
            <div className="mt-1 max-h-64 overflow-auto rounded border border-border/70 bg-card px-3 py-2 text-xs text-foreground">
              {step.technicalDetails && step.technicalDetails.length > 0 && (
                <ul className="space-y-1 text-muted-foreground">
                  {step.technicalDetails.map((detail) => (
                    <li key={detail}>{detail}</li>
                  ))}
                </ul>
              )}
              {step.payload && (
                <pre className="mt-2 whitespace-pre-wrap break-words border-t border-border/70 pt-2">
                  {step.payload}
                </pre>
              )}
            </div>
          )}
        </div>
      )}
    </li>
  );
}

export function RunTimeline({
  events,
  tasks,
  loading,
}: {
  events: Event[];
  tasks: SessionTask[];
  loading?: boolean;
}) {
  const steps = useMemo(() => {
    const merged = [...eventSteps(events), ...taskSteps(tasks, events)];
    merged.sort((a, b) => a.order - b.order || a.ts.localeCompare(b.ts));
    return merged;
  }, [events, tasks]);

  if (loading) {
    return <div className="p-4 text-sm text-muted-foreground">Loading the run…</div>;
  }

  if (steps.length === 0) {
    return (
      <div className="p-4 text-sm text-muted-foreground">Nothing has run in this session yet.</div>
    );
  }

  return (
    <ol className="relative space-y-4 border-l border-border/70 py-4 pl-2.5">
      {steps.map((step) => (
        <TimelineRow key={step.key} step={step} />
      ))}
    </ol>
  );
}
