"use client";

// Cost: what the session spent and why. Leads with spend split by cost category
// (uncached prompt, cache read, cache write, output), then spend over time, the
// tool calls and sandbox time behind it, what started the turns, and what the
// context holds now. See knowledge/ui/session-cost.md.
//
// Decision: the agent and model are in the session header, and the model can
// change from turn to turn, so this page does not repeat them as a definition
// block. A Models table appears only when more than one model ran; the run setup
// the header does not show (harness, agent revision, identity, fork origin) is a
// one-line footer.
//
// Every number is read from GET /v1/sessions/{id}/cost and the context report;
// nothing on this page can change the session.

import Link from "next/link";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { useHarness } from "@/hooks/use-harnesses";
import { useSessionContextReport, useSessionCost } from "@/hooks/use-sessions";
import { Skeleton } from "@/components/ui/skeleton";
import { getDisplayName } from "@/lib/entity-lifecycle";
import type { SessionCost } from "@/lib/api/types";
import { CostCategories, promptTokens } from "@/components/session/cost/cost-categories";
import { CostChart } from "@/components/session/cost/cost-chart";
import {
  ModelsCard,
  SandboxCard,
  SourcesCard,
  ToolsCard,
} from "@/components/session/cost/cost-activity";
import { ContextCard } from "@/components/session/cost/context-card";
import { formatCompact, formatShare } from "@/components/session/cost/cost-shared";
import { useSessionContext } from "../session-context";

/** Spend below this is too small for a caching notice to matter. */
const NOTICE_MIN_USD = 1;
/** Prompt read from cache below this share counts as poorly cached. */
const LOW_CACHE_SHARE = 0.2;
/** One origin above this share of spend is named in the notice. */
const DOMINANT_SOURCE_SHARE = 0.8;

export default function CostPage() {
  const ui = useUiTranslations();
  const { sessionId, isActive } = useSessionContext();
  const { data: cost, isLoading } = useSessionCost(sessionId, isActive);
  const { data: report } = useSessionContextReport(sessionId);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 p-6">
        {isLoading ? (
          <Skeleton className="h-72 w-full" />
        ) : cost ? (
          <>
            <SpendNotice cost={cost} sessionId={sessionId} />
            <CostCategories
              cost={cost}
              modelLabel={cost.models.length === 1 ? cost.models[0].model : undefined}
            />
            <CostChart cost={cost} />
            <div className="grid gap-6 lg:grid-cols-2">
              <ToolsCard cost={cost} />
              <div className="flex flex-col gap-6">
                <SandboxCard sandboxes={cost.sandboxes} />
                {cost.sources.length > 0 ? <SourcesCard cost={cost} /> : null}
              </div>
            </div>
            {cost.models.length > 1 ? <ModelsCard cost={cost} /> : null}
          </>
        ) : (
          <p className="border border-border bg-card p-8 text-sm text-muted-foreground">
            {ui("Spend is not available for this session.")}
          </p>
        )}
        {report && report.sections.length > 0 ? <ContextCard report={report} /> : null}
        <RunSetup />
      </div>
    </div>
  );
}

/** A warning when most spend is prompt that was not read from cache. */
function SpendNotice({ cost, sessionId }: { cost: SessionCost; sessionId: string }) {
  const { displayLocale: locale } = useDisplayLocale();
  const ui = useUiTranslations();
  const prompt = promptTokens(cost);
  const cached = cost.categories.find((c) => c.key === "cache_read")?.tokens ?? 0;
  const uncachedUsd = cost.categories.find((c) => c.key === "uncached_input")?.usd ?? 0;
  if (
    cost.total_usd < NOTICE_MIN_USD ||
    prompt === 0 ||
    cached / prompt >= LOW_CACHE_SHARE ||
    uncachedUsd / cost.total_usd < 0.5
  ) {
    return null;
  }
  const top = cost.sources[0];
  const turns = cost.sources.reduce((s, x) => s + x.turns, 0);
  const named =
    top?.kind === "schedule" && top.usd / cost.total_usd >= DOMINANT_SOURCE_SHARE
      ? ui("{value0} of {value1} turns came from the schedule “{value2}”.", {
          value0: top.turns.toLocaleString(locale),
          value1: turns.toLocaleString(locale),
          value2: top.name ?? top.kind,
        })
      : "";
  return (
    <div className="flex flex-wrap items-start gap-3 border border-warning/35 bg-warning/[0.07] px-4 py-3">
      <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-warning" aria-hidden />
      <div className="min-w-0 flex-1 text-sm">
        <div className="font-medium">
          {ui("{value0} of spend is prompt that was not read from cache", {
            value0: formatShare(uncachedUsd, cost.total_usd, locale),
          })}
        </div>
        <p className="mt-0.5 leading-relaxed">
          {ui(
            "Each model call sent about {value0} prompt tokens, and {value1} of them were read from cache.",
            {
              value0: formatCompact(prompt / Math.max(1, cost.model_calls), locale),
              value1: formatShare(cached, prompt, locale),
            },
          )}{" "}
          {named}
        </p>
      </div>
      <Link
        href={`/sessions/${sessionId}/trace`}
        className="shrink-0 border border-input bg-card px-2.5 py-1.5 text-xs font-medium shadow-xs hover:bg-muted"
      >
        {ui("See the turns in Trace")}
      </Link>
    </div>
  );
}

/** The run setup the header does not show. */
function RunSetup() {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { session, agent, agentId } = useSessionContext();
  const { data: harness } = useHarness(session?.harness_id ?? "");
  const { data: identity } = useVirtualUser(session?.virtual_user_id ?? undefined);
  if (!session) return null;

  const parts: React.ReactNode[] = [];
  if (harness) {
    parts.push(
      <span key="harness">
        {ui("Harness")}{" "}
        <Link href={`/harnesses/${harness.id}`} className="text-foreground hover:underline">
          {getDisplayName(harness, displayLocale)}
        </Link>
      </span>,
    );
  }
  if (agentId && session.agent_revision != null) {
    parts.push(
      <span key="agent">
        {ui("Agent")}{" "}
        <Link href={`/agents/${agentId}`} className="text-foreground hover:underline">
          {getDisplayName(agent, displayLocale)}
        </Link>
        , {ui("revision")} <span className="font-mono">{session.agent_revision}</span>
      </span>,
    );
  }
  parts.push(
    <span key="identity">
      {ui("Identity")}{" "}
      {session.virtual_user_id ? (
        <Link
          href={`/virtual-users/${session.virtual_user_id}`}
          className="text-foreground hover:underline"
        >
          {identity ? getDisplayName(identity, displayLocale) : session.virtual_user_id}
        </Link>
      ) : (
        <span className="text-foreground">
          {session.owner
            ? ui("{value0} principal", { value0: session.owner.kind })
            : ui("Interactive user")}
        </span>
      )}
    </span>,
  );
  if (session.forked_from_session_id) {
    parts.push(
      <span key="fork">
        {ui("Forked from")}{" "}
        <Link
          href={`/sessions/${session.forked_from_session_id}/trace`}
          className="font-mono text-foreground hover:underline"
        >
          {session.forked_from_session_id}
        </Link>
        {session.forked_from_sequence != null
          ? ` ${ui("at #")}${session.forked_from_sequence}`
          : ""}
      </span>,
    );
  }

  return (
    <footer className="flex flex-wrap items-center gap-x-3 gap-y-1 px-0.5 text-xs text-muted-foreground">
      {parts.map((part, i) => (
        <span key={i} className="flex items-center gap-3">
          {i > 0 ? <span aria-hidden>·</span> : null}
          {part}
        </span>
      ))}
    </footer>
  );
}
