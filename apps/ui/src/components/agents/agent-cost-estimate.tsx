"use client";

import { AgentCostPlanner } from "./agent-cost-planner";
import { useState } from "react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useQuery } from "@tanstack/react-query";
import { Calculator } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Drawer,
  DrawerContent,
  DrawerHeader,
  DrawerTitle,
  DrawerDescription,
} from "@/components/ui/drawer";
import { useDefaultModel, useModels } from "@/hooks/use-providers";
import { useHarnesses } from "@/hooks/use-harnesses";
import { useOrg } from "@/providers/org-provider";
import { previewAgent } from "@/lib/api/agents";
import type { PreviewAgentRequest } from "@/lib/api/types";
import { modelLabel } from "@/lib/model-selection";
import {
  COST_WORKLOADS,
  estimateDefinitionTokens,
  estimateTaskCost,
  formatEstimateUsd,
  scaleWorkload,
  resolveEstimateModel,
  type CostWorkload,
} from "@/lib/agent-cost-estimate";
import { cn } from "@/lib/utils";

interface AgentCostEstimateProps {
  modelId: string;
  maxOutputTokens: string;
  previewRequest: PreviewAgentRequest;
}

export function AgentCostEstimate(props: AgentCostEstimateProps) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="outline" onClick={() => setOpen(true)}>
        <Calculator className="size-4" />
        {ui("Estimate cost")}
      </Button>
      <Drawer open={open} onOpenChange={setOpen}>
        <DrawerContent className="w-full overflow-y-auto sm:max-w-4xl">
          <DrawerHeader className="pr-6">
            <DrawerTitle>{ui("What will this agent cost?")}</DrawerTitle>
            <DrawerDescription>
              {ui("Choose the kind of work and how often you’ll use this agent.")}
            </DrawerDescription>
          </DrawerHeader>
          {open && <AgentCostPlanner {...props} />}
        </DrawerContent>
      </Drawer>
    </>
  );
}

const FIELDS: { key: keyof CostWorkload; label: string; hint: string; min: number; max: number }[] =
  [
    {
      key: "calls",
      label: "Model calls per task",
      hint: "Include follow-up calls after tools.",
      min: 1,
      max: 100,
    },
    {
      key: "messageTokens",
      label: "Starting input tokens",
      hint: "Your message and any starting context.",
      min: 0,
      max: 1_000_000,
    },
    {
      key: "outputTokens",
      label: "Output tokens per call",
      hint: "Include reasoning and tool-call arguments.",
      min: 1,
      max: 1_000_000,
    },
    {
      key: "toolResultTokens",
      label: "Tool result tokens per call",
      hint: "New tool results added before the next call.",
      min: 0,
      max: 1_000_000,
    },
  ];

function toFields(workload: CostWorkload) {
  return Object.fromEntries(FIELDS.map(({ key }) => [key, String(workload[key])])) as Record<
    keyof CostWorkload,
    string
  >;
}

function planningCost(value: number, locale: string) {
  const format = new Intl.NumberFormat(locale, { style: "currency", currency: "USD" });
  if (value > 0 && value < 0.01) return `<${format.format(0.01)}`;
  return format.format(value);
}

const TASK_DESCRIPTIONS: Record<string, string> = {
  quick: "Answer a question or write a short reply",
  standard: "Find information and put together an answer",
  deep: "Explore a topic and compare several sources",
};

export function AgentCostCalculator({
  modelId,
  maxOutputTokens,
  previewRequest,
}: AgentCostEstimateProps) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const money = (value: number) => planningCost(value, displayLocale);
  const preciseMoney = (value: number) => formatEstimateUsd(value, displayLocale);
  const number = (value: number) => value.toLocaleString(displayLocale);
  const { currentOrg } = useOrg();
  const models = useModels();
  const defaultModel = useDefaultModel();
  const harnesses = useHarnesses();
  const preview = useQuery({
    queryKey: ["agent-cost-preview", currentOrg?.public_id, previewRequest],
    queryFn: () => previewAgent(previewRequest),
    enabled: !!currentOrg,
    staleTime: 30_000,
    retry: false,
  });
  const [preset, setPreset] = useState<string>("standard");
  const [fields, setFields] = useState(() => toFields(COST_WORKLOADS[1]));
  const [tasksPerDay, setTasksPerDay] = useState("10");
  const resolution = resolveEstimateModel(modelId, previewRequest.harness_id, harnesses.data ?? []);
  const model = resolution?.modelId
    ? models.data?.find((item) => item.id === resolution.modelId)
    : resolution
      ? defaultModel.data
      : undefined;
  const modelQuery = resolution?.modelId ? models : defaultModel;
  const needsHarness = !modelId && !!previewRequest.harness_id;
  const loading =
    preview.isPending || modelQuery.isLoading || (needsHarness && harnesses.isLoading);
  const failed = preview.isError || modelQuery.isError || (needsHarness && harnesses.isError);
  const validWorkload = FIELDS.every(
    ({ key, min, max }) =>
      fields[key].trim() !== "" &&
      Number.isInteger(Number(fields[key])) &&
      Number(fields[key]) >= min &&
      Number(fields[key]) <= max,
  );
  const workload = Object.fromEntries(
    FIELDS.map(({ key }) => [key, Number(fields[key])]),
  ) as unknown as CostWorkload;
  const volume = Number(tasksPerDay);
  const validVolume =
    tasksPerDay.trim() !== "" && Number.isInteger(volume) && volume >= 0 && volume <= 1_000_000;
  const definitionTokens = preview.data ? estimateDefinitionTokens(preview.data) : 0;
  const outputLimit = maxOutputTokens.trim() ? Number(maxOutputTokens) : undefined;
  const estimate = (scale: number) =>
    !loading && !failed && preview.data && validWorkload
      ? estimateTaskCost(
          model?.profile,
          definitionTokens,
          scaleWorkload(workload, scale),
          outputLimit,
        )
      : null;
  const expected = estimate(1);
  const low = estimate(0.5);
  const high = estimate(2);
  const usable = expected && !expected.exceedsContext;
  const range =
    low && high && !low.exceedsContext && !high.exceedsContext
      ? `${money(low.totalCost)} – ${money(high.totalCost)}`
      : null;
  const unavailable = !model
    ? ui("Choose a model for this agent before estimating its cost.")
    : !model.profile?.cost
      ? ui("We don’t have prices for this model yet. Try another model to see an estimate.")
      : !validWorkload
        ? ui("Check the numbers under Advanced assumptions, or choose a task to reset them.")
        : expected?.exceedsContext
          ? ui(
              "This task is too large for the selected model. Try a smaller task or adjust Advanced assumptions.",
            )
          : ui("We can’t estimate this setup yet. Check the model and the agent’s output limit.");

  return (
    <div className="space-y-6 pb-4">
      <div className="border bg-card p-5" aria-live="polite" aria-atomic="true">
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
          <span className="font-medium uppercase tracking-wide">
            {ui("Estimated AI cost · USD")}
          </span>
        </div>
        {loading ? (
          <p className="py-6 text-sm text-muted-foreground">{ui("Calculating your estimate…")}</p>
        ) : failed ? (
          <div className="space-y-3 py-4">
            <p role="alert" className="text-sm text-destructive">
              {ui("We couldn’t load your estimate. Please try again.")}
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                void preview.refetch();
                void modelQuery.refetch();
                if (needsHarness) void harnesses.refetch();
              }}
            >
              {ui("Retry")}
            </Button>
          </div>
        ) : usable ? (
          <>
            <div className="mt-4 flex flex-wrap items-baseline gap-x-2">
              <span className="text-4xl font-semibold tracking-tight tabular-nums">
                {money(expected.totalCost)}
              </span>
              <span className="text-sm text-muted-foreground">{ui("/ task")}</span>
            </div>
            <p className="mt-2 text-sm text-muted-foreground">
              {range
                ? ui("{range} for lighter to heavier tasks", { range })
                : ui("Larger tasks may need a different model.")}
            </p>
            <div className="mt-5 grid grid-cols-2 gap-4 border-t pt-4">
              <div>
                <p className="text-xs text-muted-foreground">{ui("Monthly cost")}</p>
                <p className="mt-1 text-xl font-semibold tabular-nums">
                  {validVolume ? money(expected.totalCost * volume * 30) : "—"}
                </p>
                <p className="text-xs text-muted-foreground">
                  {validVolume
                    ? ui("{count} tasks per month", { count: number(volume * 30) })
                    : ui("Enter tasks per day below")}
                </p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">{ui("Allow for variation")}</p>
                <p className="mt-1 text-sm font-medium tabular-nums">
                  {validVolume && range && low && high
                    ? `${money(low.totalCost * volume * 30)} – ${money(high.totalCost * volume * 30)}`
                    : "—"}
                </p>
                <p className="text-xs text-muted-foreground">
                  {ui("For simpler or more involved tasks")}
                </p>
              </div>
            </div>
          </>
        ) : (
          <p role="status" className="py-4 text-sm text-muted-foreground">
            {unavailable}
          </p>
        )}
        {model && (
          <p className="mt-4 break-words text-xs text-muted-foreground">
            {ui("Using {model}", { model: model.display_name })}
          </p>
        )}
      </div>

      <section className="space-y-3" aria-label={ui("Task assumptions")}>
        <div>
          <h3 className="text-sm font-semibold">{ui("What does a task look like?")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {ui("One task is one request, such as “research this company.”")}
          </p>
        </div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
          {COST_WORKLOADS.map((item) => (
            <button
              key={item.id}
              type="button"
              aria-pressed={preset === item.id}
              onClick={() => {
                setPreset(item.id);
                setFields(toFields(item));
              }}
              className={cn(
                "border p-3 text-left text-sm transition-colors hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring",
                preset === item.id ? "border-primary bg-primary/5" : "border-border",
              )}
            >
              <span className="block font-medium">{ui(item.name)}</span>
              <span className="mt-1 block text-xs text-muted-foreground">
                {ui(TASK_DESCRIPTIONS[item.id])}
              </span>
            </button>
          ))}
        </div>
        <div className="space-y-1.5 border-t pt-4">
          <Label htmlFor="cost-volume">{ui("Tasks per day")}</Label>
          <Input
            id="cost-volume"
            type="number"
            min={0}
            max={1_000_000}
            step={1}
            value={tasksPerDay}
            aria-invalid={!validVolume}
            aria-describedby="cost-volume-hint"
            onChange={(event) => setTasksPerDay(event.target.value)}
            className="sm:w-40"
          />
          <p
            id="cost-volume-hint"
            className={cn("text-xs", validVolume ? "text-muted-foreground" : "text-destructive")}
          >
            {validVolume
              ? ui("Monthly cost assumes 30 days of use. Each task starts a new conversation.")
              : ui("Enter a whole number from 0 to 1,000,000.")}
          </p>
        </div>
      </section>

      <p className="text-xs leading-relaxed text-muted-foreground">
        {ui(
          "A guide, not a fixed price or spending limit. Includes AI usage only. Paid tools, hosting and other service fees are extra. Trying this calculator is free and won’t run or change your agent.",
        )}
      </p>

      <details className="border-t pt-4">
        <summary className="cursor-pointer text-sm font-medium focus-visible:outline-2 focus-visible:outline-ring">
          {ui("Advanced assumptions")}
        </summary>
        <div className="mt-4 space-y-5">
          <p className="text-xs text-muted-foreground">
            {ui(
              "The task choices fill in these assumptions for you. Adjust them if you know how much work your agent usually does.",
            )}{" "}
            {model && modelLabel(model)}
            {resolution && ` · ${ui(resolution.source)}`}.
          </p>
          <div className="grid gap-4 sm:grid-cols-2">
            {FIELDS.map(({ key, label, hint, min, max }) => {
              const invalid =
                fields[key].trim() === "" ||
                !Number.isInteger(Number(fields[key])) ||
                Number(fields[key]) < min ||
                Number(fields[key]) > max;
              return (
                <div key={key} className="space-y-1.5">
                  <Label htmlFor={`cost-${key}`}>{ui(label)}</Label>
                  <Input
                    id={`cost-${key}`}
                    type="number"
                    min={min}
                    max={max}
                    step={1}
                    value={fields[key]}
                    aria-invalid={invalid}
                    aria-describedby={`cost-${key}-hint`}
                    onChange={(event) => {
                      setPreset("custom");
                      setFields({ ...fields, [key]: event.target.value });
                    }}
                  />
                  <p
                    id={`cost-${key}-hint`}
                    className={cn(
                      "text-xs",
                      invalid ? "text-destructive" : "text-muted-foreground",
                    )}
                  >
                    {invalid
                      ? ui("Enter a whole number from {min} to {max}.", {
                          min: number(min),
                          max: number(max),
                        })
                      : ui(hint)}
                  </p>
                </div>
              );
            })}
          </div>

          {usable && (
            <section className="space-y-3 border-t pt-4" aria-label={ui("Cost breakdown")}>
              <h3 className="text-sm font-semibold">{ui("Where the cost comes from")}</h3>
              <dl className="space-y-2 text-sm">
                <div className="flex justify-between gap-4">
                  <dt className="text-muted-foreground">
                    {ui("Input · {count} tokens", { count: number(expected.inputTokens) })}
                  </dt>
                  <dd className="tabular-nums">{preciseMoney(expected.inputCost)}</dd>
                </div>
                <div className="flex justify-between gap-4">
                  <dt className="text-muted-foreground">
                    {ui("Output · {count} tokens", { count: number(expected.outputTokens) })}
                  </dt>
                  <dd className="tabular-nums">{preciseMoney(expected.outputCost)}</dd>
                </div>
              </dl>
              <p className="text-xs text-muted-foreground">
                {ui(
                  "Includes roughly {count} tokens of instructions, tools and message overhead on every call. Previous outputs and tool results accumulate in the conversation.",
                  { count: number(definitionTokens) },
                )}
              </p>
              {expected.tiered && (
                <p className="text-xs text-muted-foreground">
                  {ui("Long-context pricing applies to one or more calls.")}
                </p>
              )}
              {(expected.outputCapped || high?.outputCapped) && (
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "Output is capped at the configured agent or model limit in affected scenarios.",
                  )}
                </p>
              )}
            </section>
          )}

          <div className="border-t pt-4 text-xs leading-relaxed text-muted-foreground">
            <div className="space-y-2">
              <p>
                {ui(
                  "Planning estimate, not a quote or spending limit. Lighter and heavier scenarios use half to twice the model calls, output and tool results. This is a sensitivity range, not a confidence interval or a maximum.",
                )}
              </p>
              <p>
                {ui(
                  "Uses catalog token prices and approximate text sizing, with no cache discount. Actual costs depend on the task, retries, reasoning, cache reads/writes and provider pricing. Excludes sandbox time, paid tools, media, extra models or subagents, and platform fees.",
                )}
              </p>
              <p>
                {ui(
                  "Uses the current agent draft. Estimator inputs do not change the agent. Run representative tasks in Playground and compare their recorded usage before setting a budget.",
                )}
              </p>
            </div>
          </div>
        </div>
      </details>
    </div>
  );
}
