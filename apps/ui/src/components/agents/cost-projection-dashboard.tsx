"use client";
import { useMemo, useState } from "react";
import { z } from "zod";
import {
  planningNumberInput,
  useCostPlanningDraft,
} from "@/components/chat-mode/use-cost-planning-draft";
import { useChatPageRows } from "@/components/chat-mode/page-view-state";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import type { AgentCostPlan } from "@/lib/api/agents";
import type { ModelProfile } from "@/lib/api/types";
import { formatPlanningUsd, projectTaskMix, type PlanningTask } from "@/lib/agent-cost-projection";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { VolumeControl, ProjectionChart } from "./cost-projection-controls";
import { OperatingCostEditor } from "./operating-cost-editor";
import {
  projectOperatingCosts,
  type AgentCostPricing,
  type OperatingCostLine,
} from "@/lib/agent-operating-cost";
import type { ModelCostAdjustments } from "@/lib/agent-cost-estimate";
const ASSUMPTIONS = [
  ["calls", "Model calls per task", 1, 100],
  ["messageTokens", "Starting input tokens", 0, 1_000_000],
  ["outputTokens", "Output tokens per call", 1, 1_000_000],
  ["toolResultTokens", "Tool result tokens per call", 0, 1_000_000],
] as const;

export function CostProjectionDashboard({
  plan,
  profile,
  definitionTokens,
  outputLimit,
  modelName,
  pricing = { status: "ready", modelBilling: "external", rates: [] },
}: {
  plan: AgentCostPlan;
  profile?: ModelProfile;
  definitionTokens: number;
  outputLimit?: number;
  modelName?: string;
  pricing?: AgentCostPricing;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const money = (value: number) => formatPlanningUsd(value, displayLocale);
  const [tasks, setTasks] = useState<PlanningTask[]>(() =>
    plan.tasks.map((t) => ({
      name: t.name,
      description: t.description,
      assumption: t.assumption,
      calls: t.calls,
      messageTokens: t.message_tokens,
      outputTokens: t.output_tokens,
      toolResultTokens: t.tool_result_tokens,
      weight: 33,
    })),
  );
  const [mode, setMode] = useState("tasks");
  const [volume, setVolume] = useState("100");
  const [users, setUsers] = useState("50");
  const [perUser, setPerUser] = useState("2");
  const [selected, setSelected] = useState(0);
  const [lines, setLines] = useState<OperatingCostLine[]>([]);
  const [adjustments, setAdjustments] = useState<ModelCostAdjustments>({
    cacheReadPercent: 0,
    cacheWritePercent: 0,
    retryPercent: 0,
  });
  const schema = useMemo(
    () =>
      z.strictObject({
        tasks: z
          .array(
            z.strictObject({
              calls: z.number(),
              messageTokens: z.number(),
              outputTokens: z.number(),
              toolResultTokens: z.number(),
              weight: z.number().int().min(0).max(100),
            }),
          )
          .length(plan.tasks.length)
          .describe(
            "In displayed task order: " +
              plan.tasks
                .map((task) => task.name)
                .join(", ")
                .slice(0, 2000),
          ),
        mode: z.enum(["tasks", "users"]),
        volume: planningNumberInput,
        users: planningNumberInput,
        perUser: planningNumberInput,
        selected: z
          .number()
          .int()
          .min(0)
          .max(plan.tasks.length - 1),
        adjustments: z.strictObject({
          cacheReadPercent: planningNumberInput,
          cacheWritePercent: planningNumberInput,
          retryPercent: planningNumberInput,
        }),
      }),
    [plan.tasks],
  );
  const draft = useCostPlanningDraft(
    "projection",
    schema,
    {
      tasks: tasks.map(({ calls, messageTokens, outputTokens, toolResultTokens, weight }) => ({
        calls,
        messageTokens,
        outputTokens,
        toolResultTokens,
        weight,
      })),
      mode: mode as "tasks" | "users",
      volume,
      users,
      perUser,
      selected,
      adjustments: {
        cacheReadPercent: Number.isNaN(adjustments.cacheReadPercent)
          ? ""
          : String(adjustments.cacheReadPercent),
        cacheWritePercent: Number.isNaN(adjustments.cacheWritePercent)
          ? ""
          : String(adjustments.cacheWritePercent),
        retryPercent: Number.isNaN(adjustments.retryPercent)
          ? ""
          : String(adjustments.retryPercent),
      },
    },
    (next) => {
      setTasks((previous) => previous.map((task, index) => ({ ...task, ...next.tasks[index] })));
      setMode(next.mode);
      setVolume(next.volume);
      setUsers(next.users);
      setPerUser(next.perUser);
      setSelected(next.selected);
      setAdjustments({
        cacheReadPercent:
          next.adjustments.cacheReadPercent === ""
            ? NaN
            : Number(next.adjustments.cacheReadPercent),
        cacheWritePercent:
          next.adjustments.cacheWritePercent === ""
            ? NaN
            : Number(next.adjustments.cacheWritePercent),
        retryPercent:
          next.adjustments.retryPercent === "" ? NaN : Number(next.adjustments.retryPercent),
      });
    },
  );
  // Re-resolve host rates rather than retaining a stale copied price after refresh.
  const pricedLines = lines.map((line) => {
    if (!line.rateId) return line;
    const rate =
      pricing.status === "ready" ? pricing.rates.find((r) => r.id === line.rateId) : undefined;
    return { ...line, usdPerUnit: rate?.usdPerUnit };
  });
  const valid = (v: string, max: number) =>
    v.trim() !== "" && Number.isInteger(Number(v)) && Number(v) >= 0 && Number(v) <= max;
  const validVolume =
    mode === "tasks" ? valid(volume, 1_000_000) : valid(users, 1_000_000) && valid(perUser, 1000);
  const daily = mode === "tasks" ? Number(volume) : Number(users) * Number(perUser);
  const result = projectTaskMix(tasks, profile, definitionTokens, outputLimit, adjustments);
  const operating =
    validVolume && result
      ? projectOperatingCosts(result.typical, pricing.modelBilling, pricedLines, daily)
      : null;
  useChatPageRows(
    "Cost projection",
    [
      {
        per_task_usd: operating?.total.perTask,
        daily_usd: operating?.total.daily,
        monthly_usd: operating?.total.monthly,
        fixed_monthly_usd: operating?.total.fixedMonthly,
        model_billing: pricing.modelBilling,
      },
    ],
    { loading: pricing.status === "loading", error: pricing.status === "error" },
  );
  useChatPageRows(
    "Cost plan assumptions",
    tasks.map((task, index) => ({
      index,
      name: task.name,
      description: task.description,
      assumption: task.assumption,
      calls: task.calls,
      message_tokens: task.messageTokens,
      output_tokens: task.outputTokens,
      tool_result_tokens: task.toolResultTokens,
      weight: task.weight,
    })),
  );
  const usable =
    validVolume &&
    result?.typical != null &&
    operating?.total.monthly != null &&
    pricing.status === "ready";
  const extra = projectOperatingCosts(
    0,
    pricing.modelBilling,
    pricedLines,
    validVolume ? daily : 0,
  );
  const fixedMonthly = extra?.total.fixedMonthly ?? 0;
  const extraPerTask = extra?.total.perTask ?? 0;
  const change = (index: number, patch: Partial<PlanningTask>) =>
    setTasks(tasks.map((task, i) => (i === index ? { ...task, ...patch } : task)));
  const greatest = result?.rows.reduce(
    (best, row) =>
      (row.scenarios[1]?.totalCost ?? 0) * row.share >
      (best.scenarios[1]?.totalCost ?? 0) * best.share
        ? row
        : best,
    result.rows[0],
  );
  return (
    <div className="space-y-6">
      {draft.notice}
      <section className="border bg-card p-5" aria-label={ui("Cost projection")}>
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {ui("Estimated operating cost · USD")}
        </p>
        <div className="mt-4 grid grid-cols-1 gap-5 sm:grid-cols-3" aria-live="polite">
          {[
            ["Average per task", usable ? operating.total.perTask : null],
            ["Daily cost", usable ? operating.total.daily : null],
            ["Monthly cost", usable ? operating.total.monthly : null],
          ].map(([label, value]) => (
            <div key={String(label)}>
              <p className="text-xs text-muted-foreground">{ui(String(label))}</p>
              <p className="mt-2 break-words text-2xl font-semibold tabular-nums">
                {value != null ? money(Number(value)) : "—"}
              </p>
            </div>
          ))}
        </div>
        <p className="mt-4 text-xs text-muted-foreground">
          {ui("30-day month · Each task starts a new conversation")}
          {modelName && ` · ${ui("Using {model}", { model: modelName })}`}
        </p>
        {!!pricing.creditLabel && operating && (
          <div className="mt-4 grid gap-3 border-t pt-4 sm:grid-cols-2">
            <p className="text-sm">
              {pricing.creditLabel} {ui("/ month")}:{" "}
              <strong>
                {pricing.status === "ready" && operating.credits.monthly != null
                  ? money(operating.credits.monthly)
                  : "—"}
              </strong>
            </p>
            <p className="text-sm">
              {ui("External bills")} {ui("/ month")}:{" "}
              <strong>
                {operating.external.monthly != null ? money(operating.external.monthly) : "—"}
              </strong>
            </p>
          </div>
        )}
        {pricing.status !== "ready" && (
          <p role="status" className="mt-3 text-sm">
            {ui(
              pricing.status === "loading"
                ? "Loading service prices…"
                : "Service prices are unavailable. Reopen the estimator to retry.",
            )}
          </p>
        )}
        {!!operating?.unknown.length && (
          <p role="status" className="mt-3 text-sm">
            {ui("Prices needed for: {items}", {
              items: operating.unknown.map((item) => ui(item)).join(", "),
            })}
          </p>
        )}
        {!extra && (
          <p role="alert" className="mt-3 text-sm">
            {ui("Check resource quantities and prices. Use nonnegative numbers.")}
          </p>
        )}
        <p className="mt-3 text-xs text-muted-foreground">
          {ui(
            "Per-task cost excludes fixed monthly fees. Daily cost includes one thirtieth of monthly fees. Only included resources are estimated.",
          )}
        </p>
        {!profile?.cost ? (
          <p role="status" className="mt-3 text-sm">
            {ui("We don’t have prices for this model yet. Try another model to see an estimate.")}
          </p>
        ) : result?.typical == null ? (
          <p role="status" className="mt-3 text-sm">
            {ui(
              "Check task assumptions and select at least one task. Oversized tasks cannot be priced.",
            )}
          </p>
        ) : null}
      </section>
      <section className="space-y-4" aria-label={ui("Usage projection")}>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant={mode === "tasks" ? "default" : "outline"}
            aria-pressed={mode === "tasks"}
            onClick={() => setMode("tasks")}
          >
            {ui("By tasks")}
          </Button>
          <Button
            size="sm"
            variant={mode === "users" ? "default" : "outline"}
            aria-pressed={mode === "users"}
            onClick={() => setMode("users")}
          >
            {ui("By users")}
          </Button>
        </div>
        {mode === "tasks" ? (
          <VolumeControl
            label="Tasks per day"
            id="projection-tasks"
            value={volume}
            setValue={setVolume}
            max={1_000_000}
            sliderMax={10000}
          />
        ) : (
          <div className="grid gap-4 sm:grid-cols-2">
            <VolumeControl
              label="Active users"
              id="projection-users"
              value={users}
              setValue={setUsers}
              max={1_000_000}
              sliderMax={10000}
            />
            <VolumeControl
              label="Tasks per user per day"
              id="projection-frequency"
              value={perUser}
              setValue={setPerUser}
              max={1000}
              sliderMax={100}
            />
          </div>
        )}
        {!validVolume && (
          <p role="alert" className="text-sm text-destructive">
            {ui("Enter a whole number within the displayed limits.")}
          </p>
        )}
      </section>
      {usable && operating.total.perTask != null && (
        <section
          className="space-y-3 border bg-muted/20 p-5"
          aria-label={ui("Monthly spend as usage grows")}
        >
          <h3 className="text-sm font-semibold">{ui("Monthly spend as usage grows")}</h3>
          <ProjectionChart
            typical={result.typical! + extraPerTask}
            low={result.low == null ? null : result.low + extraPerTask}
            high={result.high == null ? null : result.high + extraPerTask}
            daily={daily}
            fixedMonthly={fixedMonthly}
          />
          <p className="text-xs text-muted-foreground">
            {result.low != null && result.high != null
              ? ui("Monthly planning range: {low} – {high}", {
                  low: money((result.low + extraPerTask) * daily * 30 + fixedMonthly),
                  high: money((result.high + extraPerTask) * daily * 30 + fixedMonthly),
                })
              : ui("Larger tasks may need a different model.")}
          </p>
          {daily > 0 && greatest && (
            <p className="text-xs">
              {ui("Largest model cost driver: {task}", { task: greatest.task.name })}
            </p>
          )}
        </section>
      )}
      <OperatingCostEditor
        lines={pricedLines}
        rates={pricing.rates}
        onChange={setLines}
        dailyTasks={validVolume ? daily : null}
      />
      <section className="space-y-3" aria-label={ui("Task mix")}>
        <div>
          <h3 className="text-sm font-semibold">{ui("Typical tasks for your agent")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {ui(
              "Suggested by AI, not measured usage. Equal mix to start; adjust relative frequency or set a task to zero.",
            )}
          </p>
        </div>
        {tasks.map((task, index) => (
          <div key={index} role="group" aria-label={task.name} className="space-y-3 border p-4">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0">
                <h4 className="break-words text-sm font-medium">{task.name}</h4>
                <p className="mt-1 break-words text-xs text-muted-foreground">{task.description}</p>
              </div>
              <span className="shrink-0 text-sm tabular-nums">
                {result?.rows[index].scenarios[1] &&
                !result.rows[index].scenarios[1]?.exceedsContext
                  ? money(result.rows[index].scenarios[1]!.totalCost)
                  : "—"}
              </span>
            </div>
            <div className="flex items-center gap-3">
              <Label className="shrink-0 text-xs" htmlFor={`task-share-${index}`}>
                {ui("Frequency")}
              </Label>
              <input
                id={`task-share-${index}`}
                type="range"
                min={0}
                max={100}
                value={task.weight}
                onChange={(e) => change(index, { weight: Number(e.target.value) })}
                className="min-w-0 flex-1 accent-primary"
              />
              <span className="w-12 text-right text-xs tabular-nums">
                {new Intl.NumberFormat(displayLocale, {
                  style: "percent",
                  maximumFractionDigits: 0,
                }).format(result?.rows[index].share ?? 0)}
              </span>
            </div>
            <p className="break-words text-xs leading-relaxed text-muted-foreground">
              {task.assumption}
            </p>
          </div>
        ))}
      </section>
      <details className="border-t pt-4">
        <summary className="cursor-pointer text-sm font-medium">
          {ui("Advanced assumptions")}
        </summary>
        <div className="mt-4 space-y-4">
          <Label htmlFor="edit-cost-task">{ui("Edit task")}</Label>
          <select
            id="edit-cost-task"
            value={selected}
            onChange={(e) => setSelected(Number(e.target.value))}
            className="w-full border bg-background p-2 text-sm"
          >
            {tasks.map((t, i) => (
              <option key={i} value={i}>
                {t.name}
              </option>
            ))}
          </select>
          <Label htmlFor="cost-task-example">{ui("Task example")}</Label>
          <Input
            id="cost-task-example"
            value={tasks[selected].description}
            onChange={(e) => change(selected, { description: e.target.value })}
            maxLength={400}
          />
          <p className="text-xs text-muted-foreground">
            {ui(
              "Changing the example does not recalculate assumptions. Use the planning input above to generate a new estimate.",
            )}
          </p>
          <div className="grid gap-4 sm:grid-cols-2">
            {ASSUMPTIONS.map(([key, label, min, max]) => (
              <div key={key} className="space-y-2">
                <Label htmlFor={`plan-${key}`}>{ui(label)}</Label>
                <Input
                  id={`plan-${key}`}
                  type="number"
                  min={min}
                  max={max}
                  value={Number.isNaN(tasks[selected][key]) ? "" : tasks[selected][key]}
                  aria-invalid={
                    !Number.isInteger(tasks[selected][key]) ||
                    tasks[selected][key] < min ||
                    tasks[selected][key] > max
                  }
                  onChange={(e) =>
                    change(selected, {
                      [key]: e.target.value === "" ? NaN : Number(e.target.value),
                    })
                  }
                />
              </div>
            ))}
          </div>
          <div className="grid gap-4 sm:grid-cols-3">
            {(
              [
                ["retryPercent", "Extra model spend for retries (%)"],
                ["cacheReadPercent", "Cached input reads (%)"],
                ["cacheWritePercent", "Cache writes (%)"],
              ] as const
            ).map(([key, label]) => (
              <div key={key} className="space-y-2">
                <Label htmlFor={`adjust-${key}`}>{ui(label)}</Label>
                <Input
                  id={`adjust-${key}`}
                  type="number"
                  min={0}
                  max={100}
                  value={Number.isNaN(adjustments[key]) ? "" : adjustments[key]}
                  onChange={(e) =>
                    setAdjustments({
                      ...adjustments,
                      [key]: e.target.value === "" ? NaN : Number(e.target.value),
                    })
                  }
                />
              </div>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            {ui(
              "Cache shares apply to input tokens across all tasks and must total at most 100%. Retry allowance increases model spend only; repeated tools and sandbox time belong in resource quantities.",
            )}
          </p>
          <p className="text-xs text-muted-foreground">
            {ui(
              "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Catalog prices and model limits apply. Cache savings require explicit assumptions.",
            )}
          </p>
          {result?.rows.some((r) => r.scenarios.some((s) => s?.outputCapped)) && (
            <p className="text-xs text-muted-foreground">
              {ui("Output is capped at the configured agent or model limit in affected scenarios.")}
            </p>
          )}
        </div>
      </details>
      <p className="text-xs leading-relaxed text-muted-foreground">
        {ui(
          "Planning scenarios, not quotes or spending limits. The shaded range varies model workload; resource assumptions stay fixed. Add paid tools, sandbox time, media, subagents and fees to include them. No tasks are executed and your agent is not changed.",
        )}
      </p>
    </div>
  );
}
