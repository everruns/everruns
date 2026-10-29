"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Sparkles, RotateCcw } from "lucide-react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";
import { useModels, useDefaultModel } from "@/hooks/use-providers";
import { useHarnesses } from "@/hooks/use-harnesses";
import { planAgentCosts, previewAgent } from "@/lib/api/agents";
import type { PreviewAgentRequest } from "@/lib/api/types";
import { estimateDefinitionTokens, resolveEstimateModel } from "@/lib/agent-cost-estimate";
import { CostProjectionDashboard } from "./cost-projection-dashboard";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AgentCostCalculator } from "./agent-cost-estimate";
import { useAgentCostPricing } from "@/hooks/use-agent-cost-pricing";

interface Props {
  modelId: string;
  maxOutputTokens: string;
  previewRequest: PreviewAgentRequest;
}
export function AgentCostPlanner(props: Props) {
  const { currentOrg } = useOrg();
  const { displayLocale } = useLocale();
  // Reset generated assumptions and local edits whenever their provenance changes.
  return <Planner key={JSON.stringify([currentOrg?.public_id, displayLocale, props])} {...props} />;
}
function Planner(props: Props) {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const { displayLocale } = useLocale();
  const [manual, setManual] = useState(false);
  const [guidance, setGuidance] = useState("");
  const [submitted, setSubmitted] = useState("");
  const models = useModels();
  const defaults = useDefaultModel();
  const harnesses = useHarnesses();
  const resolution = resolveEstimateModel(
    props.modelId,
    props.previewRequest.harness_id,
    harnesses.data ?? [],
  );
  const model = resolution?.modelId
    ? models.data?.find((m) => m.id === resolution.modelId)
    : resolution
      ? defaults.data
      : undefined;
  const modelQuery = resolution?.modelId ? models : defaults;
  const pricing = useAgentCostPricing(model);
  const needsHarness = !props.modelId && !!props.previewRequest.harness_id;
  const preview = useQuery({
    queryKey: ["agent-cost-preview", currentOrg?.public_id, props.previewRequest],
    queryFn: () => previewAgent(props.previewRequest),
    enabled: !!currentOrg,
    staleTime: 30_000,
    retry: false,
  });
  const plan = useQuery({
    queryKey: [
      "agent-cost-plan",
      currentOrg?.public_id,
      props.previewRequest,
      submitted,
      displayLocale,
    ],
    queryFn: () => planAgentCosts(props.previewRequest, submitted, displayLocale),
    enabled: !!currentOrg && !manual,
    staleTime: 10 * 60_000,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  return (
    <div className="space-y-5 pb-6">
      <div className="flex flex-wrap items-center gap-2">
        <Button variant={manual ? "outline" : "default"} onClick={() => setManual(false)}>
          <Sparkles className="size-4" />
          {ui("Smart estimate")}
        </Button>
        <Button variant={manual ? "default" : "outline"} onClick={() => setManual(true)}>
          {ui("Manual calculator")}
        </Button>
      </div>
      {manual ? (
        <AgentCostCalculator {...props} />
      ) : (
        <>
          <div className="space-y-2">
            <Label htmlFor="cost-guidance">{ui("What should we plan for? (optional)")}</Label>
            <div className="flex gap-2">
              <Input
                id="cost-guidance"
                maxLength={4000}
                value={guidance}
                onChange={(e) => setGuidance(e.target.value)}
                placeholder={ui("Infer typical tasks from this agent")}
              />
              <Button
                variant="outline"
                disabled={plan.isFetching}
                onClick={() => {
                  if (guidance === submitted) void plan.refetch();
                  else setSubmitted(guidance);
                }}
              >
                <RotateCcw className="size-4" />
                {ui("Update estimate")}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              {ui(
                "AI suggests tasks from your agent. Usage sliders update instantly without another AI call.",
              )}
            </p>
          </div>
          {plan.isFetching ||
          preview.isPending ||
          modelQuery.isLoading ||
          (needsHarness && harnesses.isLoading) ? (
            <div role="status" className="border bg-muted/30 p-8 text-sm">
              {ui("Understanding your agent and estimating typical tasks…")}
            </div>
          ) : plan.isError ? (
            <div role="alert" className="space-y-2 border p-5">
              <p>
                {ui(
                  "Smart estimates are unavailable right now. Retry or use the manual calculator.",
                )}
              </p>
              <Button variant="outline" onClick={() => void plan.refetch()}>
                {ui("Retry")}
              </Button>
            </div>
          ) : plan.data &&
            preview.data &&
            !preview.isError &&
            !modelQuery.isError &&
            !(needsHarness && harnesses.isError) ? (
            <CostProjectionDashboard
              key={plan.dataUpdatedAt}
              plan={plan.data}
              profile={model?.profile}
              definitionTokens={estimateDefinitionTokens(preview.data)}
              outputLimit={props.maxOutputTokens.trim() ? Number(props.maxOutputTokens) : undefined}
              modelName={model?.display_name}
              pricing={pricing}
            />
          ) : (
            <div role="status">
              {ui("We couldn’t load your estimate. Please try again.")}{" "}
              <Button
                variant="outline"
                onClick={() => {
                  void preview.refetch();
                  void models.refetch();
                  void defaults.refetch();
                  void harnesses.refetch();
                }}
              >
                {ui("Retry")}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
