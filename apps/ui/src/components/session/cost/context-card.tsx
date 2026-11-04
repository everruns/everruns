"use client";

// "Context now": what the next model call will send, by section, with the
// per-source list folded behind one summary line.

import { useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { SessionContextReport } from "@/lib/api/types";
import { Badge } from "@/components/ui/badge";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { formatTokens } from "@/lib/formatting";
import { CostCard, formatShare } from "./cost-shared";

const SECTION_COLORS: Record<string, string> = {
  system_prompt: "bg-muted-foreground",
  tools: "bg-violet-500",
  rules: "bg-emerald-600",
  skills: "bg-amber-500",
  mcp: "bg-pink-500",
  subagents: "bg-sky-500",
  plugins: "bg-cyan-600",
  conversation: "bg-orange-500",
};
const SOURCES_SHOWN = 7;

export function ContextCard({ report }: { report: SessionContextReport }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  const [all, setAll] = useState(false);
  const total = report.estimated_input_tokens;
  const window = report.context_window_tokens;
  const percent = window && window > 0 ? Math.min(100, (total / window) * 100) : 0;
  const contributions = [...report.contributions].sort((a, b) => b.tokens - a.tokens);
  const sectionLabels = new Map(report.sections.map((s) => [s.key, s.label]));
  const largest = contributions[0];
  const shown = all ? contributions : contributions.slice(0, SOURCES_SHOWN);

  return (
    <CostCard title={ui("Context now")} meta={ui("What the next model call will send")}>
      <div className="p-4">
        <div className="mb-2 flex items-center justify-between gap-3 text-sm">
          <span className="flex items-center gap-2">
            <span className="font-medium">
              {window
                ? ui("{value0}% full", { value0: Math.round(percent) })
                : ui("Estimated usage")}
            </span>
            {percent >= 80 ? (
              <Badge variant="outline" className="border-warning/35 bg-warning/10 text-warning">
                {ui("Near the limit")}
              </Badge>
            ) : null}
          </span>
          <span className="text-muted-foreground tabular-nums">
            {formatTokens(total)}
            {window ? ui(" / {value0}", { value0: formatTokens(window) }) : ""}
          </span>
        </div>
        <div className="flex h-2 gap-0.5 bg-muted">
          {report.sections.map((section) =>
            section.tokens > 0 ? (
              <div
                key={section.key}
                className={SECTION_COLORS[section.key] ?? "bg-muted-foreground"}
                style={{ flexGrow: section.tokens, minWidth: 3 }}
                title={ui("{value0}: {value1} tokens", {
                  value0: section.label,
                  value1: section.tokens.toLocaleString(locale),
                })}
              />
            ) : null,
          )}
        </div>
        <div className="mt-4 grid gap-x-8 gap-y-2 sm:grid-cols-2">
          {report.sections.map((section) => (
            <div key={section.key} className="flex items-center justify-between gap-3 text-sm">
              <span className="flex min-w-0 items-center gap-2">
                <span
                  className={`h-2.5 w-2.5 shrink-0 ${SECTION_COLORS[section.key] ?? "bg-muted-foreground"}`}
                />
                <span className="truncate">{section.label}</span>
                <span className="text-xs text-muted-foreground">
                  {ui("{value0} items", { value0: section.items })}
                </span>
              </span>
              <span className="text-muted-foreground tabular-nums">
                {section.tokens.toLocaleString(locale)}
              </span>
            </div>
          ))}
        </div>

        {contributions.length > 0 ? (
          <div className="mt-4 border-t border-border pt-3">
            <button
              type="button"
              className="flex w-full items-center justify-between gap-3 text-left text-sm"
              aria-expanded={open}
              onClick={() => setOpen((v) => !v)}
            >
              <span className="flex items-center gap-1 font-medium">
                {open ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                {ui("Sources")}
              </span>
              <span className="truncate text-xs text-muted-foreground">
                {ui("{value0} sources · largest: {value1} {value2}", {
                  value0: contributions.length,
                  value1: largest.label,
                  value2: formatShare(largest.tokens, total, locale),
                })}
              </span>
            </button>
            {open ? (
              <>
                <table className="mt-2 w-full text-sm">
                  <thead>
                    <tr className="border-b border-border text-xs text-muted-foreground">
                      <th className="py-2 text-left font-medium">{ui("Source")}</th>
                      <th className="px-4 py-2 text-left font-medium">{ui("Section")}</th>
                      <th className="px-4 py-2 text-right font-medium">{ui("Tokens")}</th>
                      <th className="py-2 text-right font-medium">{ui("Share")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {shown.map((c) => (
                      <tr
                        key={`${c.section_key}:${c.source_id}`}
                        className="border-b border-border/60 last:border-b-0"
                      >
                        <td className="max-w-0 truncate py-1.5">{c.label}</td>
                        <td className="px-4 py-1.5 text-xs text-muted-foreground">
                          {sectionLabels.get(c.section_key) ?? c.section_key}
                        </td>
                        <td className="px-4 py-1.5 text-right tabular-nums">
                          {c.tokens.toLocaleString(locale)}
                        </td>
                        <td className="py-1.5 text-right text-muted-foreground tabular-nums">
                          {formatShare(c.tokens, total, locale)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                {contributions.length > SOURCES_SHOWN ? (
                  <button
                    type="button"
                    className="mt-2 text-xs text-muted-foreground underline-offset-2 hover:underline"
                    onClick={() => setAll((v) => !v)}
                  >
                    {all
                      ? ui("Show fewer")
                      : ui("Show all {value0}", { value0: contributions.length })}
                  </button>
                ) : null}
              </>
            ) : null}
          </div>
        ) : null}
      </div>
    </CostCard>
  );
}
