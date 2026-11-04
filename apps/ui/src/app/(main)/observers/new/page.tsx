"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Suspense } from "react";

import { useSearchParams } from "next/navigation";

import { usePageTitle } from "@/hooks";
import { PageContainer, PageBreadcrumb, PageMasthead } from "@/components/layout";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { ObserverForm, type ObserverFormInitial } from "@/components/observers/observer-form";
import { SCORER_CATALOG } from "@/components/observers/scorer-catalog";

function NewObserverForm() {
  const searchParams = useSearchParams();
  const agentId = searchParams.get("agent_id");

  // Prefill from "Observe this agent": scope to the agent, default sampling, and
  // seed the answer_completeness catalog scorer.
  const initial: ObserverFormInitial | undefined = agentId
    ? {
        agent_ids: [agentId],
        sampling_rate: 0.1,
        scorers: (() => {
          const template = SCORER_CATALOG.find((t) => t.key === "answer_completeness");
          if (!template) return [];
          return [
            {
              key: template.key,
              scope: "turn" as const,
              method: "llm_judge" as const,
              rubric: template.rubric,
              pass_threshold: template.pass_threshold,
            },
          ];
        })(),
      }
    : undefined;

  return <ObserverForm initial={initial} />;
}

export default function NewObserverPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("New Observer"), ui("Observers"));
  const observersEnabled = useFeatureFlag("observers");

  if (!observersEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">{ui("Observers is not enabled")}</p>
          <p className="text-sm">{ui("This feature is currently disabled.")}</p>
        </div>
      </div>
    );
  }

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Observers"), href: "/observers" }, { label: ui("New Observer") }]}
      />
      <PageMasthead title={ui("New Observer")} />
      <Suspense fallback={null}>
        <NewObserverForm />
      </Suspense>
    </PageContainer>
  );
}
