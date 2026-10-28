"use client";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { EvalsIcon } from "@/components/icons/facet-icons";
import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import { useEvals } from "@/hooks";
import { useAgents, usePageTitle } from "@/hooks";
import { LinkButton } from "@/components/ui/button";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import type { Eval, EvalTarget } from "@/lib/api/types";
import { getDisplayName, isArchivedStatus } from "@/lib/entity-lifecycle";
import {
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageMain,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";

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
  const ui = useUiTranslations();
  const label = targetLabel(ev.target, agentMap);

  return (
    <EntityCard
      className="h-full"
      title={ev.name}
      href={`/evals/${ev.id}`}
      headerActions={<EntityStatus status={ev.status} />}
    >
      <div className="space-y-2">
        {ev.description && <EntityCardDescription>{ev.description}</EntityCardDescription>}
        <div className="flex items-center gap-4 text-xs text-muted-foreground">
          {label && <span>{label}</span>}
          {ev.target?.type && (
            <Badge variant="outline" className="text-xs">
              {ev.target.type}
            </Badge>
          )}
          <span>
            {ev.case_count} {ui("cases")}
          </span>
        </div>
        {ev.last_run && (
          <div className="flex items-center gap-3 text-xs">
            <Badge variant={ev.last_run.status === "completed" ? "default" : "outline"}>
              {ev.last_run.status}
            </Badge>
            {ev.last_run.summary && (
              <span className={passRateColor(ev.last_run.summary.pass_rate)}>
                {(ev.last_run.summary.pass_rate * 100).toFixed(0)}
                {ui("% pass rate")}
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
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  usePageTitle(ui("Evals"));
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  useChatPageFilters({ status: statusTab });
  const { data: evals, isLoading, error } = useEvals({ includeArchived: true });
  const { data: agents } = useAgents({ includeArchived: false });

  const agentMap = new Map((agents ?? []).map((a) => [a.id, getDisplayName(a, displayLocale)]));
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
    { value: "active" as const, label: ui("Active") },
    { value: "archived" as const, label: ui("Archived") },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Evals") }]} />

      <PageMasthead
        icon={<EvalsIcon />}
        title={ui("Evals")}
        description={ui("Define, run, and track behavioral tests for your agents.")}
        actions={
          <LinkButton variant="accent" href="/evals/new">
            <Plus className="size-4" />
            {ui("New Eval")}
          </LinkButton>
        }
      />

      <PageControlStrip className="flex justify-end">
        <SectionTabs
          value={statusTab}
          onValueChange={(value) => setStatusTab(value as StatusTab)}
          items={statusItems.map((item) => ({ ...item, count: counts[item.value] }))}
        />
      </PageControlStrip>

      <PageMain>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={filteredEvals}
          errorMessagePrefix="Failed to load evals"
          emptyState={
            <div className="py-12 text-center">
              <p className="mb-4 text-muted-foreground">{ui("No evals yet")}</p>
              <LinkButton href="/evals/new">
                <Plus className="mr-2 size-4" />
                {ui("Create your first eval")}
              </LinkButton>
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
    </PageContainer>
  );
}
