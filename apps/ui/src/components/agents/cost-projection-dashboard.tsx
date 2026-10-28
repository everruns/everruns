"use client";
import { useState } from "react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import type { AgentCostPlan } from "@/lib/api/agents";
import type { ModelProfile } from "@/lib/api/types";
import { formatPlanningUsd, projectTaskMix, type PlanningTask } from "@/lib/agent-cost-projection";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { VolumeControl, ProjectionChart } from "./cost-projection-controls";
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
}: {
  plan: AgentCostPlan;
  profile?: ModelProfile;
  definitionTokens: number;
  outputLimit?: number;
  modelName?: string;
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
  const valid = (v: string, max: number) =>
    v.trim() !== "" && Number.isInteger(Number(v)) && Number(v) >= 0 && Number(v) <= max;
  const validVolume =
    mode === "tasks" ? valid(volume, 1_000_000) : valid(users, 1_000_000) && valid(perUser, 1000);
  const daily = mode === "tasks" ? Number(volume) : Number(users) * Number(perUser);
  const result = projectTaskMix(tasks, profile, definitionTokens, outputLimit);
  const usable = validVolume && result?.typical != null;
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
      <section className="border bg-card p-5" aria-label={ui("Cost projection")}>
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {ui("Estimated AI cost · USD")}
        </p>
        <div className="mt-4 grid grid-cols-1 gap-5 sm:grid-cols-3" aria-live="polite">
          {[
            ["Average per task", result?.typical],
            ["Daily cost", usable ? result.typical! * daily : null],
            ["Monthly cost", usable ? result.typical! * daily * 30 : null],
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
      {usable && (
        <section
          className="space-y-3 border bg-muted/20 p-5"
          aria-label={ui("Monthly spend as usage grows")}
        >
          <h3 className="text-sm font-semibold">{ui("Monthly spend as usage grows")}</h3>
          <ProjectionChart
            typical={result.typical!}
            low={result.low}
            high={result.high}
            daily={daily}
          />
          <p className="text-xs text-muted-foreground">
            {result.low != null && result.high != null
              ? ui("Monthly planning range: {low} – {high}", {
                  low: money(result.low * daily * 30),
                  high: money(result.high * daily * 30),
                })
              : ui("Larger tasks may need a different model.")}
          </p>
          {daily > 0 && greatest && (
            <p className="text-xs">
              {ui("Largest cost driver: {task}", { task: greatest.task.name })}
            </p>
          )}
        </section>
      )}
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
          <p className="text-xs text-muted-foreground">
            {ui(
              "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Current catalog prices and model limits apply. Cache discounts are not assumed.",
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
          "Planning scenarios, not quotes or spending limits. The shaded range varies task complexity, not statistical confidence. AI token costs only: paid tools, hosting, media, other agents and service fees are extra. No tasks are executed and your agent is not changed.",
        )}
      </p>
    </div>
  );
}
