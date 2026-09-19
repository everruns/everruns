"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Trash2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { createBudget, deleteBudget, listBudgets, updateBudget } from "@/lib/api/budgets";
import type {
  Budget,
  BudgetPeriod,
  BudgetStatus,
  BudgetSubjectType,
  CreateBudgetRequest,
  UpdateBudgetRequest,
} from "@/lib/api/types";

type ManagedBudgetSubjectType = Extract<BudgetSubjectType, "agent" | "agent_endpoint">;
type PeriodPreset = "none" | "1h" | "5h" | "24h" | "7d" | "30d" | "calendar_month" | "custom";

const PERIOD_PRESETS: { value: PeriodPreset; label: string }[] = [
  { value: "none", label: "No period (one-shot)" },
  { value: "1h", label: "Sliding 1 hour" },
  { value: "5h", label: "Sliding 5 hours" },
  { value: "24h", label: "Sliding 24 hours" },
  { value: "7d", label: "Sliding 7 days" },
  { value: "30d", label: "Sliding 30 days" },
  { value: "calendar_month", label: "Calendar month (UTC)" },
  { value: "custom", label: "Custom JSON…" },
];

const PERIOD_PRESET_TO_PERIOD: Record<Exclude<PeriodPreset, "none" | "custom">, BudgetPeriod> = {
  "1h": { type: "duration", seconds: 3_600 },
  "5h": { type: "duration", seconds: 5 * 3_600 },
  "24h": { type: "duration", seconds: 24 * 3_600 },
  "7d": { type: "duration", seconds: 7 * 24 * 3_600 },
  "30d": { type: "duration", seconds: 30 * 24 * 3_600 },
  calendar_month: { type: "calendar", unit: "month" },
};

const DATE_TIME_FORMAT = new Intl.DateTimeFormat("en-US", {
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZone: "UTC",
  timeZoneName: "short",
});

function formatCurrency(currency: string): string {
  switch (currency) {
    case "usd":
      return "USD";
    case "tokens":
      return "Tokens";
    case "credits":
      return "Credits";
    default:
      return currency.toUpperCase();
  }
}

function formatPeriod(period?: BudgetPeriod | null): string {
  if (!period) return "One-shot · no rollover";
  if (period.type === "duration") {
    const sec = period.seconds;
    if (sec % 86_400 === 0) return `${sec / 86_400}d sliding`;
    if (sec % 3_600 === 0) return `${sec / 3_600}h sliding`;
    if (sec % 60 === 0) return `${sec / 60}m sliding`;
    return `${sec}s sliding`;
  }
  if (period.type === "rolling") return `${period.window} sliding`;
  return `Calendar ${period.unit} · UTC`;
}

function rollingSeconds(window: string): number | null {
  const match = /^\s*(\d+)\s*([smhdw])\s*$/i.exec(window);
  if (!match) return null;
  const amount = Number(match[1]);
  let multiplier: number;
  switch (match[2].toLowerCase()) {
    case "s":
      multiplier = 1;
      break;
    case "m":
      multiplier = 60;
      break;
    case "h":
      multiplier = 3_600;
      break;
    case "d":
      multiplier = 86_400;
      break;
    case "w":
      multiplier = 604_800;
      break;
    default:
      return null;
  }
  return amount * multiplier;
}

function nextCalendarBoundary(startedAt: Date, unit: string): Date | null {
  const next = new Date(startedAt);
  switch (unit.toLowerCase()) {
    case "hour":
      next.setUTCMinutes(0, 0, 0);
      next.setUTCHours(next.getUTCHours() + 1);
      return next;
    case "day":
      next.setUTCHours(0, 0, 0, 0);
      next.setUTCDate(next.getUTCDate() + 1);
      return next;
    case "week": {
      next.setUTCHours(0, 0, 0, 0);
      const daysUntilMonday = (8 - next.getUTCDay()) % 7 || 7;
      next.setUTCDate(next.getUTCDate() + daysUntilMonday);
      return next;
    }
    case "month":
      return new Date(Date.UTC(next.getUTCFullYear(), next.getUTCMonth() + 1, 1));
    case "year":
      return new Date(Date.UTC(next.getUTCFullYear() + 1, 0, 1));
    default:
      return null;
  }
}

function getResetAt(period: BudgetPeriod, startedAt: Date): Date | null {
  if (period.type === "calendar") return nextCalendarBoundary(startedAt, period.unit);
  const seconds = period.type === "duration" ? period.seconds : rollingSeconds(period.window);
  return seconds === null ? null : new Date(startedAt.getTime() + seconds * 1_000);
}

function PeriodState({ budget }: { budget: Budget }) {
  if (!budget.period) {
    return <span>{formatPeriod(budget.period)}</span>;
  }
  if (!budget.period_started_at) {
    return <span>{formatPeriod(budget.period)} · starts on first use</span>;
  }

  const startedAt = new Date(budget.period_started_at);
  const resetAt = getResetAt(budget.period, startedAt);
  return (
    <span>
      {formatPeriod(budget.period)} · Started {DATE_TIME_FORMAT.format(startedAt)}
      {resetAt ? ` · Reset due ${DATE_TIME_FORMAT.format(resetAt)}` : ""}
    </span>
  );
}

function isMigratedAppBudget(budget: Budget): boolean {
  return budget.metadata?.converted_from === "app";
}

export function BudgetPanel({
  subjectType,
  subjectId,
  title,
  canManage,
}: {
  subjectType: ManagedBudgetSubjectType;
  subjectId: string;
  title?: string;
  canManage: boolean;
}) {
  const queryClient = useQueryClient();
  const queryKey = ["budgets", subjectType, subjectId];
  const [adding, setAdding] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  const budgetsQuery = useQuery({
    queryKey,
    queryFn: () => listBudgets({ subject_type: subjectType, subject_id: subjectId }),
  });
  const budgets = budgetsQuery.data ?? [];

  const createMutation = useMutation({
    mutationFn: (input: CreateBudgetRequest) => createBudget(input),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey });
      setAdding(false);
    },
  });
  const updateMutation = useMutation({
    mutationFn: ({ budgetId, input }: { budgetId: string; input: UpdateBudgetRequest }) =>
      updateBudget(budgetId, input),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey });
      setEditingId(null);
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (budgetId: string) => deleteBudget(budgetId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey }),
  });

  const mutationError = createMutation.error ?? updateMutation.error ?? deleteMutation.error;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        {title ? <h3 className="text-sm font-semibold">{title}</h3> : <span />}
        {canManage && !adding && (
          <Button variant="outline" size="sm" onClick={() => setAdding(true)}>
            <Plus className="size-3" />
            Add budget
          </Button>
        )}
      </div>

      {budgetsQuery.isLoading && <p className="text-sm text-muted-foreground">Loading budgets…</p>}
      {budgetsQuery.error instanceof Error && (
        <p className="text-sm text-destructive">{budgetsQuery.error.message}</p>
      )}
      {!budgetsQuery.isLoading && budgets.length === 0 && !adding && (
        <p className="text-sm text-muted-foreground">
          No budget is attached to this {subjectType === "agent" ? "agent" : "endpoint"}.
        </p>
      )}

      {budgets.map((budget) =>
        editingId === budget.id ? (
          <EditBudgetForm
            key={budget.id}
            budget={budget}
            submitting={updateMutation.isPending}
            onCancel={() => setEditingId(null)}
            onSubmit={(input) => updateMutation.mutate({ budgetId: budget.id, input })}
          />
        ) : (
          <div key={budget.id} className="space-y-2 rounded-md border p-3">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 space-y-1">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="outline">{formatCurrency(budget.currency)}</Badge>
                  <Badge variant={budget.status === "active" ? "outline" : "secondary"}>
                    {budget.status}
                  </Badge>
                  {isMigratedAppBudget(budget) && (
                    <Badge variant="secondary">Migrated App cap</Badge>
                  )}
                </div>
                <code className="block truncate text-xs text-muted-foreground">{budget.id}</code>
              </div>
              {canManage && (
                <div className="flex gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Edit budget ${budget.id}`}
                    onClick={() => setEditingId(budget.id)}
                  >
                    <Pencil className="size-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Delete budget ${budget.id}`}
                    onClick={() => deleteMutation.mutate(budget.id)}
                    disabled={deleteMutation.isPending}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              )}
            </div>
            <p className="text-sm">
              {budget.balance.toFixed(2)} of {budget.limit.toFixed(2)} {budget.currency} remaining
            </p>
            {budget.soft_limit != null && (
              <p className="text-xs text-muted-foreground">
                Soft limit at {budget.soft_limit.toFixed(2)} {budget.currency}
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              <PeriodState budget={budget} />
            </p>
          </div>
        ),
      )}

      {mutationError instanceof Error && (
        <p className="text-sm text-destructive">{mutationError.message}</p>
      )}

      {adding && (
        <NewBudgetForm
          subjectType={subjectType}
          subjectId={subjectId}
          submitting={createMutation.isPending}
          onCancel={() => setAdding(false)}
          onSubmit={(input) => createMutation.mutate(input)}
        />
      )}
    </div>
  );
}

function parsePositiveNumber(value: string, label: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${label} must be a positive number`);
  }
  return parsed;
}

function validateSoftLimit(value: string, limit: number): number | null {
  if (!value.trim()) return null;
  const softLimit = parsePositiveNumber(value, "Soft limit");
  if (softLimit > limit) {
    throw new Error("Soft limit must not exceed the limit");
  }
  return softLimit;
}

function NewBudgetForm({
  subjectType,
  subjectId,
  submitting,
  onCancel,
  onSubmit,
}: {
  subjectType: ManagedBudgetSubjectType;
  subjectId: string;
  submitting: boolean;
  onCancel: () => void;
  onSubmit: (input: CreateBudgetRequest) => void;
}) {
  const [currency, setCurrency] = useState("usd");
  const [limit, setLimit] = useState("10");
  const [softLimit, setSoftLimit] = useState("");
  const [periodPreset, setPeriodPreset] = useState<PeriodPreset>("calendar_month");
  const [customPeriod, setCustomPeriod] = useState(
    JSON.stringify({ type: "duration", seconds: 3_600 }, null, 2),
  );
  const [error, setError] = useState<string | null>(null);

  function buildPeriod(): BudgetPeriod | null {
    if (periodPreset === "none") return null;
    if (periodPreset === "custom") {
      return JSON.parse(customPeriod) as BudgetPeriod;
    }
    return PERIOD_PRESET_TO_PERIOD[periodPreset];
  }

  function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    try {
      const parsedLimit = parsePositiveNumber(limit, "Limit");
      onSubmit({
        subject_type: subjectType,
        subject_id: subjectId,
        currency,
        limit: parsedLimit,
        soft_limit: validateSoftLimit(softLimit, parsedLimit),
        period: buildPeriod(),
      });
      setError(null);
    } catch (submitError) {
      setError(
        submitError instanceof SyntaxError
          ? "Custom period is not valid JSON"
          : submitError instanceof Error
            ? submitError.message
            : "Invalid budget",
      );
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3 rounded-md border border-dashed p-3">
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1">
          <Label htmlFor={`${subjectId}-budget-currency`}>Currency</Label>
          <Select value={currency} onValueChange={setCurrency}>
            <SelectTrigger id={`${subjectId}-budget-currency`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="usd">USD</SelectItem>
              <SelectItem value="tokens">Tokens</SelectItem>
              <SelectItem value="credits">Credits</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label htmlFor={`${subjectId}-budget-limit`}>Limit</Label>
          <Input
            id={`${subjectId}-budget-limit`}
            type="number"
            step="0.01"
            min="0"
            value={limit}
            onChange={(event) => setLimit(event.target.value)}
            required
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor={`${subjectId}-budget-soft-limit`}>Soft limit (optional)</Label>
          <Input
            id={`${subjectId}-budget-soft-limit`}
            type="number"
            step="0.01"
            min="0"
            value={softLimit}
            onChange={(event) => setSoftLimit(event.target.value)}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor={`${subjectId}-budget-period`}>Period</Label>
          <Select
            value={periodPreset}
            onValueChange={(value) => setPeriodPreset(value as PeriodPreset)}
          >
            <SelectTrigger id={`${subjectId}-budget-period`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PERIOD_PRESETS.map((preset) => (
                <SelectItem key={preset.value} value={preset.value}>
                  {preset.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      {periodPreset === "custom" && (
        <div className="space-y-1">
          <Label htmlFor={`${subjectId}-budget-custom-period`}>Custom period JSON</Label>
          <Textarea
            id={`${subjectId}-budget-custom-period`}
            value={customPeriod}
            onChange={(event) => setCustomPeriod(event.target.value)}
            rows={4}
            spellCheck={false}
            className="font-mono text-xs"
          />
        </div>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={submitting}>
          {submitting ? "Creating…" : "Create budget"}
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onCancel} disabled={submitting}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function EditBudgetForm({
  budget,
  submitting,
  onCancel,
  onSubmit,
}: {
  budget: Budget;
  submitting: boolean;
  onCancel: () => void;
  onSubmit: (input: UpdateBudgetRequest) => void;
}) {
  const [limit, setLimit] = useState(String(budget.limit));
  const [softLimit, setSoftLimit] = useState(
    budget.soft_limit == null ? "" : String(budget.soft_limit),
  );
  const [status, setStatus] = useState<BudgetStatus>(budget.status);
  const [error, setError] = useState<string | null>(null);

  function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    try {
      const parsedLimit = parsePositiveNumber(limit, "Limit");
      onSubmit({
        limit: parsedLimit,
        soft_limit: validateSoftLimit(softLimit, parsedLimit),
        status,
      });
      setError(null);
    } catch (submitError) {
      setError(submitError instanceof Error ? submitError.message : "Invalid budget");
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3 rounded-md border border-dashed p-3">
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="space-y-1">
          <Label htmlFor={`${budget.id}-edit-limit`}>Limit</Label>
          <Input
            id={`${budget.id}-edit-limit`}
            type="number"
            step="0.01"
            min="0"
            value={limit}
            onChange={(event) => setLimit(event.target.value)}
            required
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor={`${budget.id}-edit-soft-limit`}>Soft limit (optional)</Label>
          <Input
            id={`${budget.id}-edit-soft-limit`}
            type="number"
            step="0.01"
            min="0"
            value={softLimit}
            onChange={(event) => setSoftLimit(event.target.value)}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor={`${budget.id}-edit-status`}>Status</Label>
          <Select value={status} onValueChange={(value) => setStatus(value as BudgetStatus)}>
            <SelectTrigger id={`${budget.id}-edit-status`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="active">Active</SelectItem>
              <SelectItem value="paused">Paused</SelectItem>
              <SelectItem value="disabled">Disabled</SelectItem>
              <SelectItem value="exhausted">Exhausted</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        Currency and rollover period stay fixed. Replace the budget to change them.
      </p>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={submitting}>
          {submitting ? "Saving…" : "Save budget"}
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onCancel} disabled={submitting}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
