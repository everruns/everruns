"use client";
import { useMemo, useState } from "react";

import { useEvals } from "@/hooks";
import { useAgents, usePageTitle } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { EntityCard } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { ClipboardCheck, Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import type { Eval, EvalTarget } from "@/lib/api/types";
import { getDisplayName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import {
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMain,
  PageMasthead,
  PageColumns,
  PageRail,
  RailSection,
  SectionTabs,
} from "@/components/layout";
import { cn } from "@/lib/utils";

type StatusTab = "active" | "archived";

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
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const { data: evals, isLoading, error } = useEvals({ includeArchived: true });
  const { data: agents } = useAgents({ includeArchived: false });

  const agentMap = new Map((agents ?? []).map((a) => [a.id, getDisplayName(a)]));
  const counts = useMemo(() => {
    const list = evals ?? [];
    const archived = list.filter((ev) => isArchivedStatus(ev.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [evals]);
  const filteredEvals = useMemo(
    () =>
      (evals ?? []).filter((ev) =>
        statusTab === "archived" ? isArchivedStatus(ev.status) : !isArchivedStatus(ev.status),
      ),
    [evals, statusTab],
  );
  const statusItems = [
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Evals" }]} />

      <PageMasthead
        icon={<ClipboardCheck />}
        title="Evals"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Define, run, and track behavioral tests for your agents."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <Button variant="accent" nativeButton={false} render={<Link href="/evals/new" />}>
            <Plus className="size-4" />
            New Eval
          </Button>
        }
      />

      <PageControlStrip className="flex justify-end">
        <SectionTabs
          value={statusTab}
          onValueChange={(value) => setStatusTab(value as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={filteredEvals}
            errorMessagePrefix="Failed to load evals"
            emptyState={
              <div className="py-12 text-center">
                <p className="mb-4 text-muted-foreground">No evals yet</p>
                <Button nativeButton={false} render={<Link href="/evals/new" />}>
                  <Plus className="mr-2 size-4" />
                  Create your first eval
                </Button>
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
        </PageMain>

        <PageRail>
          <RailSection label="Status">
            <div className="flex flex-col gap-1.5 text-[13px]">
              {statusItems.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => setStatusTab(item.value)}
                  className={cn(
                    "flex items-center justify-between transition-colors hover:text-foreground",
                    statusTab === item.value ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <span>{item.label}</span>
                  <span className="text-muted-foreground">{counts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredEvals.length} of {counts.all} {pluralize(counts.all, "eval")}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            View archived →
          </button>
        )}
      </PageFooter>
    </PageContainer>
  );
}
