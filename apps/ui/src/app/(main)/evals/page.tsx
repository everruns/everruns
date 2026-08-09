"use client";

import { useEvals } from "@/hooks";
import { useAgents, usePageTitle } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { EntityCard } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import type { Eval, EvalTarget } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";

function passRateColor(rate: number): string {
  if (rate >= 0.9) return "text-green-600";
  if (rate >= 0.7) return "text-yellow-600";
  return "text-red-600";
}

function targetLabel(target?: EvalTarget, agentMap?: Map<string, string>): string | undefined {
  if (!target) return undefined;
  switch (target.type) {
    case "session": {
      const agentName = target.agent_id ? agentMap?.get(target.agent_id) : undefined;
      return agentName ?? target.agent_id ?? undefined;
    }
    case "app":
      return `App: ${target.app_id}`;
    default:
      return undefined;
  }
}

function EvalCard({ eval: ev, agentMap }: { eval: Eval; agentMap: Map<string, string> }) {
  const label = targetLabel(ev.target, agentMap);

  return (
    <EntityCard
      className="h-full"
      title={ev.name}
      href={`/evals/${ev.id}`}
      copyValue={ev.id}
      headerActions={
        <Badge variant={ev.status === "active" ? "default" : "secondary"}>{ev.status}</Badge>
      }
    >
      <div className="space-y-2">
        {ev.description && (
          <p className="text-sm text-muted-foreground line-clamp-2">{ev.description}</p>
        )}
        <div className="flex items-center gap-4 text-xs text-muted-foreground">
          {label && <span>{label}</span>}
          {ev.target?.type && (
            <Badge variant="outline" className="text-xs">
              {ev.target.type}
            </Badge>
          )}
          <span>{ev.case_count} cases</span>
        </div>
        {ev.last_run && (
          <div className="flex items-center gap-3 text-xs">
            <Badge variant={ev.last_run.status === "completed" ? "default" : "outline"}>
              {ev.last_run.status}
            </Badge>
            {ev.last_run.summary && (
              <span className={passRateColor(ev.last_run.summary.pass_rate)}>
                {(ev.last_run.summary.pass_rate * 100).toFixed(0)}% pass rate
              </span>
            )}
          </div>
        )}
        {ev.tags.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {ev.tags.map((tag) => (
              <Badge key={tag} variant="outline" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}
      </div>
    </EntityCard>
  );
}

export default function EvalsPage() {
  usePageTitle("Evals");
  const { data: evals, isLoading, error } = useEvals({ includeArchived: false });
  const { data: agents } = useAgents({ includeArchived: false });

  const agentMap = new Map((agents ?? []).map((a) => [a.id, getDisplayName(a)]));

  return (
    <div className="container mx-auto p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Evals</h1>
        <Link href="/evals/new">
          <Button variant="accent">
            <Plus className="w-4 h-4 mr-2" />
            New Eval
          </Button>
        </Link>
      </div>

      {/* Eval grid */}
      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={evals}
        errorMessagePrefix="Failed to load evals"
        emptyState={
          <div className="text-center py-12">
            <p className="text-muted-foreground mb-4">No evals yet</p>
            <Link href="/evals/new">
              <Button>
                <Plus className="w-4 h-4 mr-2" />
                Create your first eval
              </Button>
            </Link>
          </div>
        }
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((ev) => (
              <EvalCard key={ev.id} eval={ev} agentMap={agentMap} />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}
