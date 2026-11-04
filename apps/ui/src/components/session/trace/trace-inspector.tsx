"use client";

// Right-hand inspector of the Trace tab: facts and payloads of the selected
// step, fetched by step on demand. Batches are described from their row.

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import { useSessionTraceStep } from "@/hooks/use-session-trace";
import type { TraceBatch, TracePayload, TraceStep, TraceStepDetail } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { JsonView, formatBytes } from "./json-view";
import { CacheBar } from "./cache-bar";
import { ROLE_COLOR, ROLE_HINT, RequestSheet } from "./request-sheet";
import { ToolChanges } from "./tool-changes";
import { formatClock, formatCount, formatDuration, parseTraceError } from "./trace-model";
import { WebSearchTraceSection } from "./web-search-trace";

export type InspectorTarget =
  /** `sessionId` is set for a sub-agent's step shown inline. */
  | { type: "step"; turn: number; step: number; sessionId?: string }
  | { type: "batch"; turn: number; batch: TraceBatch };

const KIND_TITLE: Record<string, string> = {
  model: "Model call",
  answer: "Final answer",
  approval: "Approval",
  agent: "Sub-agent",
  send: "Message",
};

function Section({
  title,
  meta,
  action,
  children,
}: {
  title: string;
  meta?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="border-b px-5 py-4">
      <div className="mb-2.5 flex items-center gap-2">
        <h3 className="text-[10px] font-medium tracking-[0.12em] text-muted-foreground uppercase">
          {title}
        </h3>
        {meta && <span className="font-mono text-[11px] text-muted-foreground">{meta}</span>}
        <div className="ml-auto">{action}</div>
      </div>
      {children}
    </section>
  );
}

function Facts({ items }: { items: [string, React.ReactNode][] }) {
  return (
    <dl className="grid grid-cols-2 gap-x-4 gap-y-3 border-b px-5 py-4">
      {items.map(([label, value]) => (
        <div key={label} className="min-w-0">
          <dt className="text-xs font-medium text-muted-foreground">{label}</dt>
          <dd className="truncate font-mono text-[13px]">{value ?? "—"}</dd>
        </div>
      ))}
    </dl>
  );
}

function StatusBadge({ status }: { status: string }) {
  return (
    <span
      className={cn(
        "border px-1.5 py-0.5 text-[11px] font-medium",
        status === "success" && "border-success/30 bg-success/12 text-success",
        status === "error" && "border-destructive/40 bg-destructive/10 text-destructive",
        status !== "success" && status !== "error" && "bg-muted text-muted-foreground",
      )}
    >
      {status}
    </span>
  );
}

function Payload({
  title,
  payload,
  error,
  onLoadFull,
}: {
  title: string;
  payload: TracePayload;
  error?: boolean;
  onLoadFull?: () => void;
}) {
  const ui = useUiTranslations();
  const text =
    typeof payload.value === "string"
      ? payload.value
      : payload.value !== undefined && payload.value !== null
        ? JSON.stringify(payload.value, null, 2)
        : (payload.preview ?? "");
  return (
    <Section
      title={title}
      meta={formatBytes(payload.size_bytes)}
      action={
        <CopyButton value={text} label={ui("Copy {value0}", { value0: title.toLowerCase() })} />
      }
    >
      <JsonView
        value={payload.value ?? undefined}
        text={payload.truncated ? (payload.preview ?? "") : undefined}
        error={error}
      />
      {payload.truncated && onLoadFull && (
        <Button variant="link" size="sm" className="mt-1 h-auto p-0 text-xs" onClick={onLoadFull}>
          {ui("Cut at 50 KB · Load full")}
        </Button>
      )}
    </Section>
  );
}

/**
 * A failed step's error, read first: the provider's own message, where it came from
 * (with a link to the provider account that served a model call), then the raw error.
 */
function ErrorSection({ payload, onLoadFull }: { payload: TracePayload; onLoadFull?: () => void }) {
  const ui = useUiTranslations();
  const value = payload.value as Record<string, unknown> | string | null | undefined;
  const parsed = parseTraceError(value ?? payload.preview);
  const record = value && typeof value === "object" ? value : undefined;
  const providerId = typeof record?.provider_id === "string" ? record.provider_id : undefined;
  const providerType = typeof record?.provider === "string" ? record.provider : undefined;
  const raw =
    typeof value === "string"
      ? value
      : value !== undefined && value !== null
        ? JSON.stringify(value, null, 2)
        : (payload.preview ?? "");
  const facts: [string, React.ReactNode][] = [];
  if (providerId) {
    facts.push([
      ui("Provider"),
      <Link
        key="provider"
        href={`/models/providers/${providerId}`}
        className="text-primary hover:underline"
      >
        {providerType ?? providerId}
      </Link>,
    ]);
  }
  if (parsed?.source) facts.push([ui("Reported by"), parsed.source]);
  if (parsed?.status) facts.push([ui("Status"), parsed.status]);
  return (
    <Section
      title={ui("Error")}
      meta={formatBytes(payload.size_bytes)}
      action={<CopyButton value={raw} label={ui("Copy error")} />}
    >
      {parsed && (
        <p className="mb-3 border-l-2 border-destructive/70 pl-3 text-sm leading-[1.55] break-words">
          {parsed.message}
        </p>
      )}
      {facts.length > 0 && (
        <dl className="mb-3 grid grid-cols-2 gap-x-4 gap-y-2">
          {facts.map(([label, node]) => (
            <div key={label} className="min-w-0">
              <dt className="text-xs font-medium text-muted-foreground">{label}</dt>
              <dd className="truncate text-[13px]">{node}</dd>
            </div>
          ))}
        </dl>
      )}
      <JsonView
        value={parsed?.body ?? payload.value ?? undefined}
        text={payload.truncated ? (payload.preview ?? "") : undefined}
        error
      />
      {payload.truncated && onLoadFull && (
        <Button variant="link" size="sm" className="mt-1 h-auto p-0 text-xs" onClick={onLoadFull}>
          {ui("Cut at 50 KB · Load full")}
        </Button>
      )}
    </Section>
  );
}

export function TraceInspector({
  sessionId,
  target,
  full,
  onLoadFull,
  onPrev,
  onNext,
}: {
  sessionId: string;
  target: InspectorTarget | null;
  full: boolean;
  onLoadFull: () => void;
  onPrev?: () => void;
  onNext?: () => void;
}) {
  const ui = useUiTranslations();
  const stepTarget = target?.type === "step" ? { turn: target.turn, step: target.step } : null;
  const stepSession = (target?.type === "step" && target.sessionId) || sessionId;
  const { data, isLoading, error } = useSessionTraceStep(stepSession, stepTarget, full);

  if (!target) {
    return (
      <div className="px-5 py-6 text-sm text-muted-foreground">
        {ui("Select a step to see its input, output and raw events.")}
      </div>
    );
  }

  const header = (label: string, title: string, mono: boolean, status?: string) => (
    <div className="border-b px-5 py-4">
      <div className="flex items-center gap-2">
        <span className="font-mono text-[11px] text-muted-foreground uppercase">{label}</span>
        <div className="ml-auto flex gap-1">
          <Button
            variant="outline"
            size="icon-sm"
            className="size-6"
            onClick={onPrev}
            disabled={!onPrev}
            aria-label={ui("Previous step")}
          >
            <ChevronUp className="size-3.5" />
          </Button>
          <Button
            variant="outline"
            size="icon-sm"
            className="size-6"
            onClick={onNext}
            disabled={!onNext}
            aria-label={ui("Next step")}
          >
            <ChevronDown className="size-3.5" />
          </Button>
        </div>
      </div>
      <div className="mt-2 flex items-center gap-2">
        <h2 className={cn("min-w-0 truncate text-lg font-semibold", mono && "font-mono")}>
          {title}
        </h2>
        {status && <StatusBadge status={status} />}
      </div>
    </div>
  );

  if (target.type === "batch") {
    const b = target.batch;
    return (
      <div>
        {header(
          `Steps #${b.first_step}–#${b.last_step} · Turn ${formatCount(target.turn)}`,
          b.name,
          true,
          b.failed > 0 ? "error" : b.running > 0 ? "running" : "success",
        )}
        <Facts
          items={[
            ["Started", formatClock(b.started_at)],
            ["Wall time", formatDuration(b.wall_ms)],
            ["Succeeded", formatCount(b.succeeded)],
            ["Failed", formatCount(b.failed)],
            ["p50 / p95", `${formatDuration(b.p50_ms)} / ${formatDuration(b.p95_ms)}`],
            ["Calls", formatCount(b.count)],
          ]}
        />
        {b.failures.length > 0 && (
          <Section
            title={ui("Failed calls")}
            meta={`${b.failures.length} of ${formatCount(b.failed)}`}
          >
            <ul className="space-y-1.5 text-xs">
              {b.failures.map((f) => (
                <li key={f.step} className="flex gap-2">
                  <span className="font-mono text-muted-foreground">#{f.step}</span>
                  <span className="min-w-0 truncate font-mono">{f.target}</span>
                  <span className="ml-auto shrink-0 text-destructive">{f.result}</span>
                </li>
              ))}
            </ul>
          </Section>
        )}
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="space-y-3 px-5 py-6">
        <Skeleton className="h-4 w-1/2" />
        <Skeleton className="h-6 w-2/3" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }
  if (error || !data) {
    return (
      <div className="px-5 py-6 text-sm text-destructive">{ui("Could not load this step.")}</div>
    );
  }
  return (
    <StepDetail
      key={`${stepSession}:${data.step.turn}.${data.step.step}`}
      sessionId={stepSession}
      subAgent={stepSession !== sessionId}
      detail={data}
      header={header}
      onLoadFull={full ? undefined : onLoadFull}
    />
  );
}

function StepDetail({
  sessionId,
  subAgent,
  detail,
  header,
  onLoadFull,
}: {
  sessionId: string;
  subAgent: boolean;
  detail: TraceStepDetail;
  header: (label: string, title: string, mono: boolean, status?: string) => React.ReactNode;
  onLoadFull?: () => void;
}) {
  const ui = useUiTranslations();
  const step: TraceStep = detail.step;
  const isModel = step.kind === "model" || step.kind === "answer";
  const title = isModel
    ? (KIND_TITLE[step.kind] ?? step.kind)
    : (step.name ?? KIND_TITLE[step.kind] ?? step.kind);
  const facts: [string, React.ReactNode][] = isModel
    ? [
        ["Started", formatClock(step.started_at)],
        ["Duration", formatDuration(step.duration_ms) || "—"],
        ["Prompt tokens", step.input_tokens != null ? formatCount(step.input_tokens) : "—"],
        ["Completion tokens", step.output_tokens != null ? formatCount(step.output_tokens) : "—"],
        ["Model", step.model ?? "—"],
        ["Tool calls asked", formatCount(step.requested_tool_call_ids?.length ?? 0)],
      ]
    : [
        ["Started", formatClock(step.started_at)],
        ["Duration", formatDuration(step.duration_ms) || "—"],
        ["Call id", step.tool_call_id ?? "—"],
        ["Target", step.target ?? "—"],
      ];
  if (step.child_session_id) {
    facts.push([
      "Sub-session",
      <Link
        key="child"
        href={`/sessions/${step.child_session_id}/trace`}
        className="text-primary hover:underline"
      >
        {step.child_session_id}
      </Link>,
    ]);
  }
  const request = detail.request;
  const [sheetOpen, setSheetOpen] = useState(false);
  return (
    <div>
      {header(
        `${subAgent ? "Sub-agent · " : ""}Step #${step.step} · Turn ${formatCount(step.turn)}`,
        title,
        !isModel,
        step.status,
      )}
      <Facts items={facts} />
      {step.status === "error" && detail.output && (
        <ErrorSection payload={detail.output} onLoadFull={onLoadFull} />
      )}
      {step.narration && (
        <Section title={step.kind === "answer" ? ui("Answer") : ui("Narration")}>
          <p className="text-sm leading-[1.55] break-words whitespace-pre-wrap">{step.narration}</p>
        </Section>
      )}
      {request && (
        <Section
          title={ui("Request")}
          meta={`${formatCount(request.message_count)} messages · ${formatCount(request.tool_count)} tools`}
          action={
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={() => setSheetOpen(true)}
            >
              {ui("View full request")}
            </Button>
          }
        >
          {request.usage && <CacheBar usage={request.usage} className="mb-3" />}
          {request.tool_changes && <ToolChanges changes={request.tool_changes} />}
          <ul className="border text-xs">
            {/* When every message is new, the system prompt is already among them. */}
            {request.system_preview && request.new_from > 0 && (
              <RequestRow role="system" preview={request.system_preview} />
            )}
            {request.new_from > 0 && (
              <li className="border-t border-border/60 bg-muted/50 px-2.5 py-1.5 italic text-muted-foreground first:border-t-0">
                {formatCount(request.new_from)}{" "}
                {ui("earlier messages, unchanged since the previous call")}
              </li>
            )}
            {request.new_messages.map((m) => (
              <RequestRow key={m.index} role={m.role} preview={m.preview} isNew />
            ))}
          </ul>
          <RequestSheet
            sessionId={sessionId}
            turn={step.turn}
            step={step.step}
            open={sheetOpen}
            onOpenChange={setSheetOpen}
          />
        </Section>
      )}
      {step.name === "web_search" && (
        <WebSearchTraceSection sessionId={sessionId} detail={detail} />
      )}
      {detail.input && (
        <Payload title={ui("Input")} payload={detail.input} onLoadFull={onLoadFull} />
      )}
      {detail.output && step.status !== "error" && (
        <Payload
          title={step.status === "error" ? ui("Error") : isModel ? ui("Response") : ui("Output")}
          payload={detail.output}
          error={step.status === "error"}
          onLoadFull={onLoadFull}
        />
      )}
      <Section
        title={ui("Events")}
        meta={String(detail.events.length)}
        action={
          <Link
            href={`/sessions/${sessionId}/events`}
            className="text-xs font-medium text-primary hover:underline"
          >
            {ui("Open in Events")}
          </Link>
        }
      >
        <ul className="text-xs">
          {detail.events.map((event) => (
            <li
              key={event.id}
              className="flex gap-3 border-b border-border/60 py-1 last:border-b-0"
            >
              <span className="w-24 shrink-0 font-mono text-muted-foreground">
                {formatClock(event.ts)}
              </span>
              <span
                className={cn(
                  "min-w-0 truncate font-mono",
                  /failed|error/.test(event.type) && "text-destructive",
                )}
              >
                {event.type}
              </span>
              <span className="ml-auto shrink-0 font-mono text-muted-foreground">
                #{event.sequence}
              </span>
            </li>
          ))}
        </ul>
      </Section>
    </div>
  );
}

function RequestRow({ role, preview, isNew }: { role: string; preview: string; isNew?: boolean }) {
  const ui = useUiTranslations();
  return (
    <li
      className={cn(
        "flex items-center gap-2 border-t border-border/60 px-2.5 py-1.5 first:border-t-0",
        isNew && "bg-accent/6",
      )}
    >
      <span
        className={cn("w-[62px] shrink-0 font-mono text-[11px]", ROLE_COLOR[role] ?? "text-info")}
        title={ROLE_HINT}
      >
        {role}
      </span>
      <span className="min-w-0 flex-1 truncate">{preview || "—"}</span>
      {isNew && (
        <span className="shrink-0 border border-accent bg-accent/15 px-1 text-[10px]">
          {ui("new")}
        </span>
      )}
    </li>
  );
}
