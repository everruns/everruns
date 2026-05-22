"use client";

import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useSessionContextReport } from "@/hooks/use-sessions";
import { formatTokens } from "@/lib/formatting";
import { RefreshCw } from "lucide-react";
import { useSessionContext } from "../session-context";

const SECTION_COLORS: Record<string, string> = {
  system_prompt: "bg-zinc-500",
  tools: "bg-violet-500",
  rules: "bg-emerald-600",
  skills: "bg-amber-500",
  mcp: "bg-pink-500",
  subagents: "bg-sky-500",
  plugins: "bg-cyan-600",
  conversation: "bg-orange-500",
};

export default function SessionContextPage() {
  const { sessionId } = useSessionContext();
  const { data: report, isLoading, isFetching, refetch } = useSessionContextReport(sessionId);

  if (isLoading) {
    return (
      <div className="flex-1 overflow-auto p-6">
        <Skeleton className="mb-4 h-9 w-48" />
        <Skeleton className="h-80 w-full" />
      </div>
    );
  }

  const total = report?.estimated_input_tokens ?? 0;
  const windowTokens = report?.context_window_tokens;
  const percent =
    windowTokens && windowTokens > 0 ? Math.min(100, (total / windowTokens) * 100) : 0;
  const sectionLabels = new Map(
    (report?.sections ?? []).map((section) => [section.key, section.label]),
  );
  const contributions = [...(report?.contributions ?? [])].sort((a, b) => b.tokens - a.tokens);

  return (
    <div className="flex-1 overflow-auto p-6">
      <div className="mb-5 flex items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">Context</h2>
          <p className="text-sm text-muted-foreground">
            {report
              ? `${formatTokens(total)} estimated input tokens${windowTokens ? ` / ${formatTokens(windowTokens)} context` : ""}`
              : "No LLM context has been recorded for this session yet."}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => refetch()} disabled={isFetching}>
          <RefreshCw className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      </div>

      {report && report.sections.length > 0 ? (
        <div className="rounded-lg border bg-card p-5">
          <div className="mb-2 flex items-center justify-between text-sm">
            <span>{windowTokens ? `${Math.round(percent)}% full` : "Estimated usage"}</span>
            <span className="text-muted-foreground tabular-nums">
              {formatTokens(total)}
              {windowTokens ? ` / ${formatTokens(windowTokens)}` : ""}
            </span>
          </div>
          <div className="mb-6 flex h-2 overflow-hidden rounded-full bg-muted">
            {report.sections.map((section) => (
              <div
                key={section.key}
                className={SECTION_COLORS[section.key] ?? "bg-muted-foreground"}
                style={{ width: `${total > 0 ? (section.tokens / total) * 100 : 0}%` }}
                title={`${section.label}: ${section.tokens.toLocaleString()} tokens`}
              />
            ))}
          </div>

          <div className="space-y-3">
            {report.sections.map((section) => (
              <div key={section.key} className="flex items-center justify-between gap-4 text-sm">
                <div className="flex min-w-0 items-center gap-3">
                  <span
                    className={`h-3 w-3 rounded-sm ${SECTION_COLORS[section.key] ?? "bg-muted-foreground"}`}
                  />
                  <span>{section.label}</span>
                  <span className="text-xs text-muted-foreground">{section.items} items</span>
                </div>
                <span className="tabular-nums text-muted-foreground">
                  {section.tokens.toLocaleString()}
                </span>
              </div>
            ))}
          </div>

          {contributions.length > 0 ? (
            <div className="mt-6 border-t pt-5">
              <div className="mb-3 flex items-center justify-between gap-3 text-sm">
                <span className="font-medium">Source breakdown</span>
                <span className="text-xs text-muted-foreground">% of current context</span>
              </div>
              <div className="space-y-2">
                {contributions.map((contribution) => {
                  const contributionPercent =
                    total > 0 ? Math.round((contribution.tokens / total) * 100) : 0;
                  return (
                    <div
                      key={`${contribution.section_key}:${contribution.source_id}`}
                      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-4 text-sm"
                    >
                      <div className="flex min-w-0 items-center gap-3">
                        <span
                          className={`h-3 w-3 rounded-sm ${SECTION_COLORS[contribution.section_key] ?? "bg-muted-foreground"}`}
                        />
                        <div className="min-w-0">
                          <div className="truncate">{contribution.label}</div>
                          <div className="text-xs text-muted-foreground">
                            {sectionLabels.get(contribution.section_key) ??
                              contribution.section_key}
                          </div>
                        </div>
                      </div>
                      <div className="text-right tabular-nums text-muted-foreground">
                        <div>{contributionPercent}%</div>
                        <div className="text-xs">{contribution.tokens.toLocaleString()}</div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          ) : null}
        </div>
      ) : (
        <div className="rounded-lg border bg-card p-8 text-sm text-muted-foreground">
          No context report is available until the session makes an LLM call.
        </div>
      )}
    </div>
  );
}
