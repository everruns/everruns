"use client";

import { Suspense } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { ArrowLeft } from "lucide-react";
import { usePageTitle } from "@/hooks";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
  usePageTitle("New Observer", "Observers");
  const observersEnabled = useFeatureFlag("observers");

  if (!observersEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">Observers is not enabled</p>
          <p className="text-sm">This feature is currently disabled.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/observers"
        className="mb-6 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to Observers
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New Observer</CardTitle>
        </CardHeader>
        <CardContent>
          <Suspense fallback={null}>
            <NewObserverForm />
          </Suspense>
        </CardContent>
      </Card>
    </div>
  );
}
