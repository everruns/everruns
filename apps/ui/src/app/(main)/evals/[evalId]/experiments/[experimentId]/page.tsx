"use client";

// One experiment: variants side by side, the verdict, and cases compared one
// by one (approved mockup 02). Polls while a variant's run is still going.

import { use, useMemo } from "react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useEval, useEvalExperiment, useModels, usePageTitle } from "@/hooks";
import { EvalsIcon } from "@/components/icons/facet-icons";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Notice } from "@/components/ui/notice";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import {
  CaseComparisonTable,
  VariantTable,
  VerdictNotice,
} from "@/components/evals/experiment-comparison";
import { experimentCaseCount, experimentCost } from "@/components/evals/experiment-format";
import { formatCostUsd, formatRelativeTime } from "@/lib/formatting";
import { ApiError } from "@/lib/api/client";

export default function EvalExperimentPage({
  params,
}: {
  params: Promise<{ evalId: string; experimentId: string }>;
}) {
  const { evalId, experimentId } = use(params);
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const { data: ev } = useEval(evalId);
  const { data: experiment, isLoading, error } = useEvalExperiment(evalId, experimentId);
  const { data: models } = useModels();
  usePageTitle(experiment?.name ?? ui("Experiment"), ev?.name ?? null, ui("Evals"));

  const modelNames = useMemo(
    () => new Map((models ?? []).map((model) => [model.id, model.display_name])),
    [models],
  );

  if (error instanceof ApiError && error.status === 404) {
    return (
      <ResourceNotFound
        title={ui("Experiment not found")}
        description={ui(
          "This experiment may have been deleted, moved to another organization, or the URL may be wrong.",
        )}
        backHref="/evals?tab=experiments"
        backLabel="Back to experiments"
        resourceId={experimentId}
      />
    );
  }

  const cases = experiment ? experimentCaseCount(experiment) : null;
  const cost = experiment ? experimentCost(experiment) : null;
  const meta = experiment
    ? [
        ev ? `${ev.name} v${experiment.dataset_version}` : `v${experiment.dataset_version}`,
        cases !== null ? ui("{count} cases", { count: cases }) : null,
        experiment.trials === 1
          ? ui("1 trial")
          : ui("{count} trials", { count: experiment.trials }),
        experiment.status === "running"
          ? ui("running")
          : ui("started {when}", {
              when: formatRelativeTime(experiment.created_at, displayLocale),
            }),
        cost !== null ? ui("cost {cost}", { cost: formatCostUsd(cost) }) : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : undefined;
  const verdicts = experiment?.comparison?.verdicts ?? [];

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: ui("Evals"), href: "/evals" },
          { label: ui("Experiments"), href: "/evals?tab=experiments" },
          { label: experiment?.name ?? ui("Experiment") },
        ]}
      />
      <PageMasthead
        icon={<EvalsIcon />}
        title={experiment?.name ?? ui("Experiment")}
        description={meta}
        badges={
          experiment && experiment.status !== "completed" ? (
            <Badge variant="outline">{experiment.status}</Badge>
          ) : undefined
        }
      />
      <PageMain className="space-y-4">
        {isLoading || !experiment ? (
          <Skeleton className="h-64 w-full" />
        ) : (
          <>
            <VariantTable experiment={experiment} modelNames={modelNames} />
            {experiment.status === "running" ? (
              <Notice variant="info">
                <p className="text-sm">
                  {ui(
                    "Variants are still running. The verdict appears when every variant's run finishes.",
                  )}
                </p>
              </Notice>
            ) : (
              verdicts.map((verdict) => <VerdictNotice key={verdict.label} verdict={verdict} />)
            )}
            <CaseComparisonTable experiment={experiment} />
          </>
        )}
      </PageMain>
    </PageContainer>
  );
}
