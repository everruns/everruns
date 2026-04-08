"use client";

import { useEvals } from "@/hooks";
import { useAgents } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ExperimentalPageBadge } from "@/components/ui/experimental-badge";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import type { Eval } from "@/lib/api/types";

function passRateColor(rate: number): string {
  if (rate >= 0.9) return "text-green-600";
  if (rate >= 0.7) return "text-yellow-600";
  return "text-red-600";
}

function EvalCard({ eval: ev, agentName }: { eval: Eval; agentName?: string }) {
  return (
    <Link href={`/evals/${ev.id}`}>
      <Card className="hover:border-primary/50 transition-colors cursor-pointer h-full">
        <CardHeader className="pb-2">
          <div className="flex items-center justify-between">
            <CardTitle className="text-base truncate">{ev.name}</CardTitle>
            <Badge variant={ev.status === "active" ? "default" : "secondary"}>{ev.status}</Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          {ev.description && (
            <p className="text-sm text-muted-foreground line-clamp-2">{ev.description}</p>
          )}
          <div className="flex items-center gap-4 text-xs text-muted-foreground">
            {agentName && <span>Agent: {agentName}</span>}
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
        </CardContent>
      </Card>
    </Link>
  );
}

export default function EvalsPage() {
  const evalsEnabled = useFeatureFlag("evals");
  const { data: evals, isLoading, error } = useEvals({ includeArchived: false });
  const { data: agents } = useAgents({ includeArchived: false });

  const agentMap = new Map((agents ?? []).map((a) => [a.id, a.display_name || a.name]));

  if (!evalsEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">Evals is not enabled</p>
          <p className="text-sm">This feature is currently disabled.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold flex items-center gap-3">
          Evals
          <ExperimentalPageBadge />
        </h1>
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
              <EvalCard key={ev.id} eval={ev} agentName={agentMap.get(ev.agent_id)} />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}
