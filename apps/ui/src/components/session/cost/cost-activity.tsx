"use client";

// Tool calls, sandbox running time, what started the turns, and models: the
// usage behind the spend that is not itself a token category.

import type { CostSandbox, CostSource, SessionCost } from "@/lib/api/types";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";
import { CostCard, ShareBar, Swatch, formatSeconds, formatShare, formatUsd } from "./cost-shared";

const TOOLS_SHOWN = 8;

export function ToolsCard({ cost }: { cost: SessionCost }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const { tools } = cost;
  const rest = tools.items.slice(TOOLS_SHOWN);
  return (
    <CostCard
      title={ui("Tool calls")}
      meta={
        tools.calls > 0
          ? [
              ui("{value0} calls", { value0: tools.calls.toLocaleString(locale) }),
              tools.failed > 0
                ? ui("{value0} failed", { value0: tools.failed.toLocaleString(locale) })
                : null,
              ui("{value0} running", { value0: formatSeconds(tools.duration_ms / 1000) }),
            ]
              .filter(Boolean)
              .join(" · ")
          : undefined
      }
    >
      {tools.items.length === 0 ? (
        <p className="p-4 text-sm text-muted-foreground">{ui("No tool calls yet.")}</p>
      ) : (
        <>
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border text-xs text-muted-foreground">
                <th className="px-4 py-2 text-left font-medium">{ui("Tool")}</th>
                <th className="px-4 py-2 text-right font-medium">{ui("Calls")}</th>
                <th className="px-4 py-2 text-right font-medium">{ui("Failed")}</th>
                <th className="px-4 py-2 text-right font-medium">{ui("Time")}</th>
              </tr>
            </thead>
            <tbody>
              {tools.items.slice(0, TOOLS_SHOWN).map((tool) => (
                <tr key={tool.name} className="border-b border-border/60 last:border-b-0">
                  <td className="max-w-0 truncate px-4 py-2 font-mono text-[13px]">{tool.name}</td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {tool.calls.toLocaleString(locale)}
                  </td>
                  <td
                    className={cn(
                      "px-4 py-2 text-right tabular-nums",
                      tool.failed > 0 ? "text-destructive" : "text-muted-foreground",
                    )}
                  >
                    {tool.failed.toLocaleString(locale)}
                  </td>
                  <td className="px-4 py-2 text-right text-muted-foreground tabular-nums">
                    {formatSeconds(tool.duration_ms / 1000)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="border-t border-border px-4 py-2.5 text-xs text-muted-foreground">
            {rest.length > 0
              ? `${ui("{value0} more tools, {value1} calls.", {
                  value0: rest.length,
                  value1: rest.reduce((s, t) => s + t.calls, 0).toLocaleString(locale),
                })} `
              : ""}
            {ui("Tool results add cost by joining the prompt of every later model call.")}
          </p>
        </>
      )}
    </CostCard>
  );
}

export function SandboxCard({ sandboxes }: { sandboxes: CostSandbox[] }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  if (sandboxes.length === 0) {
    return (
      <CostCard title={ui("Sandbox")}>
        <p className="p-4 text-sm text-muted-foreground">{ui("No sandbox ran in this session.")}</p>
      </CostCard>
    );
  }
  const running = sandboxes.reduce((s, b) => s + b.running_seconds, 0);
  const paused = sandboxes.reduce((s, b) => s + b.paused_seconds, 0);
  const providers = [...new Set(sandboxes.map((b) => b.provider))].join(", ");
  return (
    <CostCard title={sandboxes.length > 1 ? ui("Sandboxes") : ui("Sandbox")} meta={providers}>
      <div className="space-y-4 p-4">
        <div className="flex flex-wrap gap-8">
          <Figure label={ui("Running")} value={formatSeconds(running)} />
          <Figure label={ui("Paused")} value={formatSeconds(paused)} />
        </div>
        {sandboxes.map((sandbox) => (
          <SandboxTimeline key={sandbox.sandbox_id} sandbox={sandbox} locale={locale} />
        ))}
        <div className="flex flex-wrap gap-4 text-xs text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <Swatch className="bg-[#b5487a] dark:bg-[#d06a98]" />
            {ui("Running")}
          </span>
          <span className="flex items-center gap-1.5">
            <Swatch className="bg-muted-foreground/30" />
            {ui("Paused")}
          </span>
          <span className="flex items-center gap-1.5">
            <Swatch className="bg-destructive/60" />
            {ui("Lost or failed")}
          </span>
        </div>
      </div>
    </CostCard>
  );
}

function Figure({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-[15px] font-medium tabular-nums">{value}</div>
    </div>
  );
}

function SandboxTimeline({ sandbox, locale }: { sandbox: CostSandbox; locale: string }) {
  const spans = sandbox.spans;
  if (spans.length === 0) return null;
  const start = new Date(spans[0].start).getTime();
  const end = new Date(spans[spans.length - 1].end).getTime();
  const total = Math.max(1, end - start);
  const color = (state: string) =>
    state === "running"
      ? "bg-[#b5487a] dark:bg-[#d06a98]"
      : state === "paused"
        ? "bg-muted-foreground/30"
        : state === "lost" || state === "failed"
          ? "bg-destructive/60"
          : "bg-muted";
  const fmt = (t: number) =>
    new Date(t).toLocaleString(locale, {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  return (
    <div>
      <div className="mb-1 flex justify-between text-xs text-muted-foreground">
        <span className="truncate">{sandbox.profile_name ?? sandbox.sandbox_id}</span>
        <span className="tabular-nums">{formatSeconds(sandbox.running_seconds)}</span>
      </div>
      <div className="flex h-3 bg-muted">
        {spans.map((span, i) => {
          const from = new Date(span.start).getTime();
          const to = new Date(span.end).getTime();
          return (
            <span
              key={i}
              className={color(span.state)}
              style={{ width: `${((to - from) / total) * 100}%` }}
              title={`${span.state}: ${formatSeconds((to - from) / 1000)}`}
            />
          );
        })}
      </div>
      <div className="mt-1 flex justify-between text-[11px] text-muted-foreground">
        <span>{fmt(start)}</span>
        <span>{spans[spans.length - 1].current ? "" : fmt(end)}</span>
      </div>
    </div>
  );
}

export function SourcesCard({ cost }: { cost: SessionCost }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const turns = cost.sources.reduce((s, x) => s + x.turns, 0);
  const label = (source: CostSource) => {
    switch (source.kind) {
      case "user":
        return ui("People");
      case "schedule":
        return ui("Schedule");
      case "agent_trigger":
        return ui("Agent trigger");
      case "channel":
        return ui("Channel");
      default:
        return source.kind;
    }
  };
  return (
    <CostCard
      title={ui("Who started the turns")}
      meta={ui("{value0} turns", { value0: turns.toLocaleString(locale) })}
    >
      <table className="w-full text-sm">
        <tbody>
          {cost.sources.map((source) => (
            <tr
              key={`${source.kind}:${source.source_id ?? ""}`}
              className="border-b border-border/60 last:border-b-0"
            >
              <td className="max-w-0 px-4 py-2">
                <div className="truncate">
                  {label(source)}
                  {source.name ? (
                    <span className="text-muted-foreground"> · {source.name}</span>
                  ) : null}
                </div>
                {source.cron || source.active === false ? (
                  <div className="text-xs text-muted-foreground">
                    {[source.cron, source.active === false ? ui("no longer runs") : null]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                ) : null}
              </td>
              <td className="px-4 py-2 text-right whitespace-nowrap text-muted-foreground tabular-nums">
                {ui("{value0} turns", { value0: source.turns.toLocaleString(locale) })}
              </td>
              <td className="px-4 py-2 text-right font-medium tabular-nums">
                {formatUsd(source.usd, locale)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </CostCard>
  );
}

/** Shown only when a session used more than one model. */
export function ModelsCard({ cost }: { cost: SessionCost }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  return (
    <CostCard title={ui("Models")}>
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-xs text-muted-foreground">
            <th className="px-4 py-2 text-left font-medium">{ui("Model")}</th>
            <th className="px-4 py-2 text-right font-medium">{ui("Calls")}</th>
            <th className="px-4 py-2 text-right font-medium">{ui("Spend")}</th>
            <th className="px-4 py-2 text-right font-medium">{ui("Share")}</th>
          </tr>
        </thead>
        <tbody>
          {cost.models.map((model) => (
            <tr
              key={`${model.provider ?? ""}:${model.model}`}
              className="border-b border-border/60 last:border-b-0"
            >
              <td className="max-w-0 truncate px-4 py-2 font-mono text-[13px]">{model.model}</td>
              <td className="px-4 py-2 text-right tabular-nums">
                {model.calls.toLocaleString(locale)}
              </td>
              <td className="px-4 py-2 text-right font-medium tabular-nums">
                {formatUsd(model.usd, locale)}
              </td>
              <td className="px-4 py-2 text-right whitespace-nowrap tabular-nums">
                <ShareBar
                  share={cost.total_usd > 0 ? model.usd / cost.total_usd : 0}
                  className="bg-foreground/60"
                />
                <span className="ml-2 inline-block w-12">
                  {formatShare(model.usd, cost.total_usd, locale)}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </CostCard>
  );
}
