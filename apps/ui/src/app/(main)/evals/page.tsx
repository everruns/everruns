"use client";

// The Evals section (approved mockup 01): one place for how agents do on test
// datasets and on real traffic. Tabs live in `?tab=` so links and refreshes
// land on the same view. Observers keep their own pages; the tab lists them so
// tests and live scoring sit side by side.

import { Suspense, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Play, Plus } from "lucide-react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";
import { EvalsIcon } from "@/components/icons/facet-icons";
import {
  useAgents,
  useAllEvalExperiments,
  useEvalRunsMany,
  useEvals,
  useObservers,
  usePageTitle,
} from "@/hooks";
import { useEvalFindings } from "@/hooks/use-health-issues";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageMain,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { getDisplayName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { buildEvalsOverview, evalAgentId } from "@/components/evals/evals-overview-data";
import { EvalsOverviewPanel } from "@/components/evals/section/evals-overview";
import { DatasetsGrid } from "@/components/evals/section/datasets-grid";
import { ExperimentsTable } from "@/components/evals/section/experiments-table";
import { FindingsList } from "@/components/evals/section/findings-list";
import { GradersList } from "@/components/evals/section/graders-list";
import { RunExperimentDialog } from "@/components/evals/run-experiment-dialog";
import { ObserverCard } from "@/components/observers/observer-card";
import { resolveEvalsTab } from "@/components/evals/section/tabs";

function EvalsSection() {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const router = useRouter();
  const searchParams = useSearchParams();
  const tab = resolveEvalsTab(searchParams.get("tab"));
  usePageTitle(ui("Evals"));
  useChatPageFilters({ tab });
  const observersEnabled = useFeatureFlag("observers");
  const [runOpen, setRunOpen] = useState(false);

  const { data: evals, isLoading, error } = useEvals({ includeArchived: true });
  const { data: agents } = useAgents({ includeArchived: false });
  const { data: experiments } = useAllEvalExperiments();
  const { data: findingsPage } = useEvalFindings();
  const { data: observers } = useObservers({
    includeArchived: false,
    enabled: observersEnabled,
  });
  const findings = useMemo(() => findingsPage?.data ?? [], [findingsPage]);

  const activeEvals = useMemo(
    () => (evals ?? []).filter((ev) => !isArchivedStatus(ev.status)),
    [evals],
  );
  const agentEvalIds = activeEvals.filter((ev) => evalAgentId(ev)).map((ev) => ev.id);
  const runsByEval = useEvalRunsMany(tab === "overview" ? agentEvalIds : []);
  const overview = useMemo(
    () =>
      buildEvalsOverview({
        evals: activeEvals,
        agents: agents ?? [],
        findings,
        experiments: experiments ?? [],
        runsByEval,
        agentName: (agent) => getDisplayName(agent, displayLocale),
      }),
    [activeEvals, agents, findings, experiments, runsByEval, displayLocale],
  );

  const selectTab = (next: string) => {
    const params = new URLSearchParams(searchParams.toString());
    if (next === "overview") params.delete("tab");
    else params.set("tab", next);
    const query = params.toString();
    router.replace(query ? `/evals?${query}` : "/evals", { scroll: false });
  };

  const items = [
    { value: "overview", label: ui("Overview") },
    { value: "datasets", label: ui("Datasets"), count: activeEvals.length },
    {
      value: "experiments",
      label: ui("Experiments"),
      count: experiments?.length,
    },
    ...(observersEnabled
      ? [
          {
            value: "observers",
            label: ui("Observers"),
            count: observers?.length,
          },
        ]
      : []),
    { value: "findings", label: ui("Findings"), count: overview.openFindings },
    { value: "graders", label: ui("Graders") },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Evals") }]} />

      <PageMasthead
        icon={<EvalsIcon />}
        title={ui("Evals")}
        description={ui("How your agents do on test datasets and on real traffic.")}
        actions={
          <>
            <LinkButton variant="outline" href="/evals/new">
              <Plus className="size-4" />
              {ui("New dataset")}
            </LinkButton>
            <Button variant="accent" onClick={() => setRunOpen(true)}>
              <Play className="size-4" />
              {ui("Run experiment")}
            </Button>
          </>
        }
      />

      <PageControlStrip>
        <SectionTabs value={tab} onValueChange={selectTab} items={items} />
      </PageControlStrip>

      <PageMain>
        {tab === "overview" && <EvalsOverviewPanel overview={overview} findings={findings} />}
        {tab === "datasets" && (
          <DatasetsGrid evals={evals} agents={agents} isLoading={isLoading} error={error} />
        )}
        {tab === "experiments" && (
          <Card>
            <CardContent className="p-0">
              <ExperimentsTable
                experiments={experiments ?? []}
                evals={evals ?? []}
                emptyText={ui("No experiments yet. Run one to compare models on the same cases.")}
              />
            </CardContent>
          </Card>
        )}
        {tab === "observers" && observersEnabled && (
          <div className="space-y-4">
            <div className="flex justify-end">
              <LinkButton variant="outline" href="/observers/new">
                <Plus className="size-4" />
                {ui("New Observer")}
              </LinkButton>
            </div>
            {(observers ?? []).length === 0 ? (
              <p className="text-sm text-muted-foreground">
                {ui("No observers yet. An observer scores a sample of live sessions.")}
              </p>
            ) : (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {(observers ?? []).map((observer) => (
                  <ObserverCard key={observer.id} observer={observer} />
                ))}
              </div>
            )}
          </div>
        )}
        {tab === "findings" && (
          <Card>
            <CardContent className="p-0">
              <FindingsList
                issues={findings}
                showTime
                emptyText={ui("No open findings. Detectors and classifiers found no problems.")}
              />
            </CardContent>
          </Card>
        )}
        {tab === "graders" && <GradersList />}
      </PageMain>

      {runOpen && (
        <RunExperimentDialog open={runOpen} onOpenChange={setRunOpen} evals={activeEvals} />
      )}
    </PageContainer>
  );
}

export default function EvalsPage() {
  return (
    <Suspense>
      <EvalsSection />
    </Suspense>
  );
}
