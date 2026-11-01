"use client";

// Every usage policy of the org in one list: who it covers, what it measures,
// over what window, and what happens at which point (notify, ask to continue,
// stop). Stored as budgets on the API; the UI calls them usage policies.

import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Trash2 } from "lucide-react";

import { formatUsd } from "@/components/costs/cost-basis";
import { Button } from "@/components/ui/button";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SegmentedControl } from "@/components/ui/segmented-control";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { useAgents } from "@/hooks/use-agents";
import { useMembers } from "@/hooks/use-members";
import { createBudget, deleteBudget, listBudgets, updateBudget } from "@/lib/api/budgets";
import {
  effectiveSteps,
  getCostSettings,
  updateCostSettings,
  type PolicyStep,
  type PolicyStepAction,
  type UsagePolicy,
} from "@/lib/api/costs";
import type { Budget, BudgetPeriod } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";

type Covers = "organization" | "org_unattended" | "agent" | "user";
type Window = "none" | "day" | "week" | "month";

const ACTION_LABELS: Record<PolicyStepAction, string> = {
  notify: "Notify",
  ask: "Ask to continue",
  stop: "Stop",
};

const ACTION_HINTS: Record<PolicyStepAction, string> = {
  notify: "Organization owners and admins get a notification.",
  ask: "The person using the agent approves before the turn continues. Unattended work stops.",
  stop: "No more model calls until the limit is raised or the window resets.",
};

const SUBJECT_LABELS: Record<string, string> = {
  organization: "Organization",
  org_unattended: "Unattended work",
  agent: "Agent",
  agent_channel: "Channel",
  agent_trigger: "Trigger",
  user: "Person",
  session: "Session",
  session_schedule: "Schedule",
  eval_run: "Eval run",
};

function policyOf(budget: Budget): UsagePolicy {
  return (budget.policy ?? {}) as UsagePolicy;
}

function windowOf(period?: BudgetPeriod | null): string {
  if (!period) return "total";
  if (period.type === "calendar") return period.unit;
  if (period.type === "duration") {
    const days = period.seconds / 86_400;
    return Number.isInteger(days) ? `${days} days` : `${period.seconds / 3_600} hours`;
  }
  return period.window;
}

export function formatLimit(budget: Pick<Budget, "limit" | "currency" | "period">): string {
  const amount =
    budget.currency === "usd" ? formatUsd(budget.limit) : `${budget.limit.toLocaleString()} tokens`;
  const window = windowOf(budget.period);
  return window === "total" ? amount : `${amount} / ${window}`;
}

function UsedBar({ budget }: { budget: Budget }) {
  const spent = Math.max(0, budget.limit - budget.balance);
  const pct = budget.limit > 0 ? Math.min(100, (spent / budget.limit) * 100) : 100;
  const steps = effectiveSteps(policyOf(budget), budget.limit, budget.soft_limit);
  const reached = steps.filter((s) => pct >= s.at_percent).pop();
  const tone =
    reached?.action === "stop"
      ? "bg-destructive"
      : reached?.action === "ask"
        ? "bg-warning"
        : "bg-foreground";
  return (
    <div className="flex items-center gap-2">
      <div className="relative h-1.5 w-32 bg-muted" aria-hidden>
        <div className={`h-full ${tone}`} style={{ width: `${pct}%` }} />
        {steps
          .filter((s) => s.at_percent < 100)
          .map((s) => (
            <div
              key={`${s.action}-${s.at_percent}`}
              className="absolute top-[-2px] h-2.5 w-px bg-warning"
              style={{ left: `${s.at_percent}%` }}
            />
          ))}
      </div>
      <span className="text-xs tabular-nums text-muted-foreground">
        {budget.currency === "usd" ? formatUsd(spent) : spent.toLocaleString()}
      </span>
    </div>
  );
}

interface PolicyForm {
  covers: Covers;
  subjectId: string;
  name: string;
  source: "all" | "managed" | "own_keys";
  currency: "usd" | "tokens";
  limit: string;
  window: Window;
  steps: PolicyStep[];
}

const DEFAULT_STEPS: PolicyStep[] = [
  { at_percent: 80, action: "notify" },
  { at_percent: 100, action: "stop" },
];

function formFromBudget(budget: Budget | null, orgId: string): PolicyForm {
  if (!budget) {
    return {
      covers: "organization",
      subjectId: orgId,
      name: "",
      source: "all",
      currency: "usd",
      limit: "",
      window: "month",
      steps: DEFAULT_STEPS,
    };
  }
  const policy = policyOf(budget);
  const window: Window =
    budget.period?.type === "calendar" && ["day", "week", "month"].includes(budget.period.unit)
      ? (budget.period.unit as Window)
      : "none";
  return {
    covers: (["organization", "org_unattended", "agent", "user"].includes(budget.subject_type)
      ? budget.subject_type
      : "organization") as Covers,
    subjectId: budget.subject_id,
    name: policy.name ?? "",
    source: policy.source ?? "all",
    currency: budget.currency === "tokens" ? "tokens" : "usd",
    limit: String(budget.limit),
    window,
    steps: effectiveSteps(policy, budget.limit, budget.soft_limit),
  };
}

function PolicyEditor({ budget, onDone }: { budget: Budget | null; onDone: () => void }) {
  const ui = useUiTranslations();
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const orgId = currentOrg?.public_id ?? "";
  const [form, setForm] = useState<PolicyForm>(() => formFromBudget(budget, orgId));
  const agents = useAgents({});
  const members = useMembers(form.covers === "user");
  const set = (patch: Partial<PolicyForm>) => setForm((f) => ({ ...f, ...patch }));

  const save = useMutation({
    mutationFn: async () => {
      const limit = Number(form.limit);
      const period: BudgetPeriod | null =
        form.window === "none" ? null : { type: "calendar", unit: form.window };
      const policy: UsagePolicy = {
        name: form.name.trim() || undefined,
        steps: form.steps,
        source: form.source,
      };
      if (budget) {
        // Measure, coverage and window are fixed once a policy exists: a
        // changed window would make its spent amount mean something else.
        return updateBudget(budget.id, { limit, policy });
      }
      return createBudget({
        subject_type: form.covers,
        subject_id:
          form.covers === "organization" || form.covers === "org_unattended"
            ? orgId
            : form.subjectId,
        currency: form.currency,
        limit,
        period,
        policy,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["budgets"] });
      onDone();
    },
  });

  const limit = Number(form.limit);
  const needsSubject = form.covers === "agent" || form.covers === "user";
  const valid =
    Number.isFinite(limit) &&
    limit > 0 &&
    (!needsSubject || form.subjectId.length > 0) &&
    form.steps.every((s) => s.at_percent > 0 && s.at_percent <= 100);

  const updateStep = (index: number, patch: Partial<PolicyStep>) =>
    set({ steps: form.steps.map((s, i) => (i === index ? { ...s, ...patch } : s)) });

  return (
    <div className="space-y-4 px-4 pb-6">
      <div className="space-y-1.5">
        <Label>{ui("Covers")}</Label>
        <SegmentedControl
          label={ui("Covers")}
          value={form.covers}
          onValueChange={(value) => !budget && set({ covers: value as Covers, subjectId: "" })}
          options={[
            { value: "organization", label: ui("Organization") },
            { value: "user", label: ui("Person") },
            { value: "agent", label: ui("Agent") },
            { value: "org_unattended", label: ui("Unattended work") },
          ]}
        />
        {budget ? (
          <p className="text-xs text-muted-foreground">
            {ui("Who a policy covers cannot change.")}
          </p>
        ) : null}
      </div>
      {form.covers === "agent" && !budget ? (
        <Select value={form.subjectId} onValueChange={(value) => set({ subjectId: value ?? "" })}>
          <SelectTrigger aria-label={ui("Agent")}>
            <SelectValue placeholder={ui("Choose an agent")} />
          </SelectTrigger>
          <SelectContent>
            {(agents.data ?? []).map((agent) => (
              <SelectItem key={agent.id} value={agent.id}>
                {agent.display_name || agent.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
      {form.covers === "user" && !budget ? (
        <Select value={form.subjectId} onValueChange={(value) => set({ subjectId: value ?? "" })}>
          <SelectTrigger aria-label={ui("Person")}>
            <SelectValue placeholder={ui("Choose a person")} />
          </SelectTrigger>
          <SelectContent>
            {(members.data ?? []).map((member) => (
              <SelectItem key={member.user_id} value={member.user_id}>
                {member.name || member.email}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
      <div className="space-y-1.5">
        <Label htmlFor="policy-name">{ui("Name (optional)")}</Label>
        <Input id="policy-name" value={form.name} onChange={(e) => set({ name: e.target.value })} />
      </div>
      <div className="space-y-1.5">
        <Label>{ui("Only count")}</Label>
        <SegmentedControl
          label={ui("Only count")}
          value={form.source}
          onValueChange={(value) => set({ source: value as PolicyForm["source"] })}
          options={[
            { value: "all", label: ui("All usage") },
            { value: "managed", label: ui("Built-in models") },
            { value: "own_keys", label: ui("Own keys and sandboxes") },
          ]}
        />
      </div>
      <div className="space-y-1.5">
        <Label>{ui("Measure")}</Label>
        <SegmentedControl
          label={ui("Measure")}
          value={form.currency}
          onValueChange={(value) => !budget && set({ currency: value as PolicyForm["currency"] })}
          options={[
            { value: "usd", label: ui("Cost") },
            { value: "tokens", label: ui("Tokens") },
          ]}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="policy-limit">{ui("Limit and window")}</Label>
        <div className="flex flex-wrap gap-2">
          <Input
            id="policy-limit"
            className="w-32"
            inputMode="decimal"
            value={form.limit}
            placeholder={form.currency === "usd" ? "$ 50" : "2000000"}
            onChange={(e) => set({ limit: e.target.value })}
          />
          <SegmentedControl
            label={ui("Window")}
            value={form.window}
            onValueChange={(value) => !budget && set({ window: value as Window })}
            options={[
              { value: "day", label: ui("Day") },
              { value: "week", label: ui("Week") },
              { value: "month", label: ui("Month") },
              { value: "none", label: ui("Total") },
            ]}
          />
        </div>
        <p className="text-xs text-muted-foreground">
          {ui("Days, weeks and months follow the organization's timezone.")}
        </p>
      </div>
      <div className="space-y-2">
        <Label>{ui("Steps")}</Label>
        {form.steps.map((step, index) => (
          <div
            key={index}
            className="grid grid-cols-[5rem_10rem_1fr_auto] items-center gap-2 border px-2 py-2"
          >
            <Input
              aria-label={ui("Percent of the limit")}
              inputMode="decimal"
              value={String(step.at_percent)}
              onChange={(e) => updateStep(index, { at_percent: Number(e.target.value) })}
            />
            <Select
              value={step.action}
              onValueChange={(value) => updateStep(index, { action: value as PolicyStepAction })}
            >
              <SelectTrigger aria-label={ui("What happens")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(ACTION_LABELS) as PolicyStepAction[]).map((action) => (
                  <SelectItem key={action} value={action}>
                    {ui(ACTION_LABELS[action])}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span className="text-xs text-muted-foreground">{ui(ACTION_HINTS[step.action])}</span>
            <Button
              size="icon"
              variant="ghost"
              aria-label={ui("Remove step")}
              onClick={() => set({ steps: form.steps.filter((_, i) => i !== index) })}
            >
              <Trash2 className="size-4" />
            </Button>
          </div>
        ))}
        {form.steps.length < 8 ? (
          <Button
            size="sm"
            variant="outline"
            onClick={() => set({ steps: [...form.steps, { at_percent: 90, action: "ask" }] })}
          >
            <Plus className="size-4" />
            {ui("Add step")}
          </Button>
        ) : null}
        <p className="text-xs text-muted-foreground">
          {ui("A policy always stops at 100% of its limit.")}
        </p>
      </div>
      {save.error ? (
        <p role="alert" className="text-sm text-destructive">
          {save.error.message}
        </p>
      ) : null}
      <div className="flex gap-2">
        <Button onClick={() => save.mutate()} disabled={!valid || save.isPending}>
          {ui(budget ? "Save usage policy" : "Create usage policy")}
        </Button>
        <Button variant="outline" onClick={onDone}>
          {ui("Cancel")}
        </Button>
      </div>
    </div>
  );
}

function TimezoneSetting({ canManage }: { canManage: boolean }) {
  const ui = useUiTranslations();
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["costs", "settings"], queryFn: getCostSettings });
  const zones = useMemo(() => {
    const supported = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
    return supported ? supported("timeZone") : ["UTC"];
  }, []);
  const save = useMutation({
    mutationFn: (timezone: string | null) => updateCostSettings({ timezone }),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["costs"] }),
  });
  const current = settings.data?.timezone ?? "UTC";
  return (
    <div className="flex flex-wrap items-center gap-3 border bg-card px-4 py-3 text-sm">
      <span className="font-medium">{ui("Organization timezone")}</span>
      <span className="text-muted-foreground">
        {ui("Calendar days, weeks and months of policies and costs start at midnight here.")}
      </span>
      <div className="ml-auto w-64">
        <Select
          value={current}
          disabled={!canManage || save.isPending}
          onValueChange={(value) => save.mutate(value === "UTC" ? null : (value ?? null))}
        >
          <SelectTrigger aria-label={ui("Organization timezone")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {["UTC", ...zones.filter((zone) => zone !== "UTC")].map((zone) => (
              <SelectItem key={zone} value={zone}>
                {zone}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}

export function UsagePolicies({ canManage }: { canManage: boolean }) {
  const ui = useUiTranslations();
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<Budget | null | undefined>(undefined);
  const budgets = useQuery({ queryKey: ["budgets", "all"], queryFn: () => listBudgets() });
  const agents = useAgents({});
  const members = useMembers();
  const remove = useMutation({
    mutationFn: (budget: Budget) => deleteBudget(budget.id),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["budgets"] }),
  });
  const subjectName = (budget: Budget): string => {
    if (budget.subject_type === "agent") {
      const agent = (agents.data ?? []).find((a) => a.id === budget.subject_id);
      return agent ? agent.display_name || agent.name : budget.subject_id;
    }
    if (budget.subject_type === "user") {
      const member = (members.data ?? []).find((m) => m.user_id === budget.subject_id);
      return member ? member.name || member.email : budget.subject_id;
    }
    if (budget.subject_type === "organization" || budget.subject_type === "org_unattended")
      return "";
    return budget.subject_id;
  };

  return (
    <div className="space-y-4">
      <TimezoneSetting canManage={canManage} />
      <section className="border bg-card">
        <div className="flex items-center justify-between gap-3 border-b px-4 py-3">
          <div>
            <h2 className="text-sm font-semibold">{ui("Usage policies")}</h2>
            <p className="text-xs text-muted-foreground">
              {ui("When several policies cover the same work, the most restrictive one wins.")}
            </p>
          </div>
          {canManage ? (
            <Button size="sm" onClick={() => setEditing(null)}>
              <Plus className="size-4" />
              {ui("New usage policy")}
            </Button>
          ) : null}
        </div>
        {budgets.isLoading ? (
          <Skeleton className="m-4 h-24" />
        ) : budgets.error ? (
          <p role="alert" className="px-4 py-3 text-sm">
            {ui("Unable to load usage policies.")}
          </p>
        ) : (budgets.data ?? []).length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            {ui("No usage policies yet. Without one, nothing limits what agents spend.")}
          </p>
        ) : (
          <ul className="divide-y">
            {(budgets.data ?? []).map((budget) => {
              const policy = policyOf(budget);
              const steps = effectiveSteps(policy, budget.limit, budget.soft_limit);
              return (
                <li
                  key={budget.id}
                  className="grid items-center gap-3 px-4 py-3 text-sm md:grid-cols-[1fr_10rem_12rem_auto]"
                >
                  <div>
                    <div>
                      <span className="font-medium">
                        {ui(SUBJECT_LABELS[budget.subject_type] ?? budget.subject_type)}
                      </span>{" "}
                      {subjectName(budget)}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {[
                        policy.name,
                        policy.source === "managed"
                          ? ui("Only built-in models")
                          : policy.source === "own_keys"
                            ? ui("Only own keys and sandboxes")
                            : null,
                        steps
                          .map((s) => `${Math.round(s.at_percent)}% ${ui(ACTION_LABELS[s.action])}`)
                          .join(" · "),
                        budget.status !== "active" ? ui("Stopped until the limit is raised") : null,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </div>
                  </div>
                  <div className="tabular-nums">{formatLimit(budget)}</div>
                  <UsedBar budget={budget} />
                  <div className="flex justify-end gap-1">
                    {canManage ? (
                      <>
                        <Button
                          size="icon"
                          variant="ghost"
                          aria-label={ui("Edit usage policy")}
                          onClick={() => setEditing(budget)}
                        >
                          <Pencil className="size-4" />
                        </Button>
                        <Button
                          size="icon"
                          variant="ghost"
                          aria-label={ui("Delete usage policy")}
                          disabled={remove.isPending}
                          onClick={() => remove.mutate(budget)}
                        >
                          <Trash2 className="size-4" />
                        </Button>
                      </>
                    ) : null}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>
      <Drawer open={editing !== undefined} onOpenChange={(open) => !open && setEditing(undefined)}>
        <DrawerContent className="w-full overflow-y-auto sm:max-w-xl">
          <DrawerHeader className="pr-6">
            <DrawerTitle>{ui(editing ? "Edit usage policy" : "New usage policy")}</DrawerTitle>
            <DrawerDescription>
              {ui(
                "Counted from the costs in your price list. A built-in model counts what your host charged for it.",
              )}
            </DrawerDescription>
          </DrawerHeader>
          {editing !== undefined ? (
            <PolicyEditor budget={editing} onDone={() => setEditing(undefined)} />
          ) : null}
        </DrawerContent>
      </Drawer>
    </div>
  );
}
