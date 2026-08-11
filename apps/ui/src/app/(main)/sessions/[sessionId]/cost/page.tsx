"use client";

// Cost — what the run consumed and what produced it (EVE-854): token totals,
// retries, the context breakdown (folded in from the retired Context tab), and
// the definition block that pins the recording to the exact agent version,
// harness, model and identity that ran it.
//
// Every number here is derived from the event stream or from read endpoints;
// nothing on this page can change the session.

import { useMemo } from "react";
import Link from "next/link";
import { Bot, Boxes, Fingerprint, RefreshCcw, Sparkles, Zap } from "lucide-react";
import { useAgentIdentity } from "@/hooks/use-agent-identities";
import { useAgentVersions } from "@/hooks/use-agents";
import { useHarness } from "@/hooks/use-harnesses";
import { useSessionContextReport } from "@/hooks/use-sessions";
import { Skeleton } from "@/components/ui/skeleton";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { getEventData } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { formatTokens } from "@/lib/formatting";
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

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <div className="text-xs uppercase tracking-[0.14em] text-muted-foreground">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums">{value}</div>
      {hint ? <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div> : null}
    </div>
  );
}

function DefinitionRow({
  icon: Icon,
  label,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4 border-b border-border/60 py-3 last:border-b-0">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Icon className="h-4 w-4" />
        {label}
      </div>
      <div className="min-w-0 text-right text-sm">{children}</div>
    </div>
  );
}

export default function CostPage() {
  const { sessionId, session, agent, agentId, llmModel, liveUsage, events } = useSessionContext();
  const { data: report, isLoading: reportLoading } = useSessionContextReport(sessionId);
  const { data: harness } = useHarness(session?.harness_id ?? "");
  const { data: versions } = useAgentVersions(agentId);
  const { data: identity } = useAgentIdentity(session?.agent_identity_id ?? undefined);

  // Retries are not a first-class event: they are the failed attempts the run
  // recovered from. Count failed turns and failed LLM calls separately so a
  // provider hiccup inside a turn is not confused with a turn that gave up.
  const { failedTurns, failedGenerations, toolFailures } = useMemo(() => {
    let failedTurns = 0;
    let failedGenerations = 0;
    let toolFailures = 0;
    for (const event of events ?? []) {
      if (getEventData(event, "turn.failed")) failedTurns += 1;
      const generation = getEventData(event, "llm.generation");
      if (generation && !generation.metadata.success) failedGenerations += 1;
      const tool = getEventData(event, "tool.completed");
      if (tool && !tool.success) toolFailures += 1;
    }
    return { failedTurns, failedGenerations, toolFailures };
  }, [events]);

  const agentVersion = useMemo(
    () => (versions ?? []).find((version) => version.id === session?.agent_version_id),
    [versions, session?.agent_version_id],
  );

  const total = report?.estimated_input_tokens ?? 0;
  const windowTokens = report?.context_window_tokens;
  const percent =
    windowTokens && windowTokens > 0 ? Math.min(100, (total / windowTokens) * 100) : 0;
  const contributions = [...(report?.contributions ?? [])].sort((a, b) => b.tokens - a.tokens);
  const sectionLabels = new Map(
    (report?.sections ?? []).map((section) => [section.key, section.label]),
  );

  return (
    <div className="flex-1 space-y-8 overflow-y-auto p-6">
      <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <Stat
          label="Prompt tokens"
          value={liveUsage ? formatTokens(liveUsage.input_tokens) : "—"}
          hint={
            liveUsage?.cache_read_tokens
              ? `${formatTokens(liveUsage.cache_read_tokens)} read from cache`
              : undefined
          }
        />
        <Stat
          label="Completion tokens"
          value={liveUsage ? formatTokens(liveUsage.output_tokens) : "—"}
        />
        <Stat
          label="Failed turns"
          value={String(failedTurns)}
          hint={failedGenerations > 0 ? `${failedGenerations} model calls errored` : undefined}
        />
        <Stat label="Tool failures" value={String(toolFailures)} />
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium text-muted-foreground">Definition</h2>
        <div className="rounded-lg border border-border bg-card px-4">
          <DefinitionRow icon={Bot} label="Agent">
            {agentId ? (
              <Link href={`/agents/${agentId}`} className="hover:underline">
                {getDisplayName(agent)}
              </Link>
            ) : (
              <span className="text-muted-foreground">None</span>
            )}
            {agentVersion ? (
              <span className="ml-2 font-mono text-xs text-muted-foreground">
                v{agentVersion.version}
              </span>
            ) : session?.agent_version_id ? (
              <span className="ml-2 font-mono text-xs text-muted-foreground">
                <EntityIdentity value={session.agent_version_id}>pinned version</EntityIdentity>
              </span>
            ) : null}
          </DefinitionRow>

          <DefinitionRow icon={Boxes} label="Harness">
            {harness ? (
              <Link href={`/harnesses/${harness.id}`} className="hover:underline">
                {getDisplayName(harness)}
              </Link>
            ) : (
              <EntityIdentity value={session?.harness_id ?? ""}>
                {session?.harness_id ?? "—"}
              </EntityIdentity>
            )}
          </DefinitionRow>

          <DefinitionRow icon={Sparkles} label="Model">
            {llmModel ? (
              <>
                {llmModel.display_name}
                <span className="ml-2 font-mono text-xs text-muted-foreground">
                  {llmModel.model_id}
                </span>
              </>
            ) : (
              <span className="text-muted-foreground">Resolved per turn</span>
            )}
          </DefinitionRow>

          <DefinitionRow icon={Fingerprint} label="Identity">
            {session?.agent_identity_id ? (
              <Link
                href={`/agent-identities/${session.agent_identity_id}`}
                className="hover:underline"
              >
                {identity ? getDisplayName(identity) : session.agent_identity_id}
              </Link>
            ) : (
              <span className="text-muted-foreground">
                {session?.owner ? `${session.owner.kind} principal` : "Interactive user"}
              </span>
            )}
          </DefinitionRow>

          {session?.forked_from_session_id ? (
            <DefinitionRow icon={RefreshCcw} label="Forked from">
              <Link
                href={`/sessions/${session.forked_from_session_id}/transcript`}
                className="font-mono text-xs hover:underline"
              >
                {session.forked_from_session_id}
              </Link>
              {session.forked_from_sequence != null ? (
                <span className="ml-2 text-xs text-muted-foreground">
                  at #{session.forked_from_sequence}
                </span>
              ) : null}
            </DefinitionRow>
          ) : null}
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Zap className="h-4 w-4 text-muted-foreground" />
          <h2 className="text-sm font-medium text-muted-foreground">Context</h2>
        </div>

        {reportLoading ? (
          <Skeleton className="h-64 w-full" />
        ) : report && report.sections.length > 0 ? (
          <div className="rounded-lg border bg-card p-5">
            <div className="mb-2 flex items-center justify-between text-sm">
              <span>{windowTokens ? `${Math.round(percent)}% full` : "Estimated usage"}</span>
              <span className="tabular-nums text-muted-foreground">
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
      </section>
    </div>
  );
}
