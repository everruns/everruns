"use client";

// AppBudgetsCard — manage budgets attached to an app or one of its channels.
// Behind the experimental `app_budgets` feature flag.
//
// Period UI exposes the common cases (sliding hours/days, calendar month,
// none). The "Custom JSON" field is the escape hatch that lets advanced
// users author any rule the backend understands (e.g. arbitrary calendar
// units, future rule extensions) without waiting for first-class form
// fields.

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { createBudget, deleteBudget, listAppBudgets } from "@/lib/api/budgets";
import type {
  App,
  AppChannel,
  Budget,
  BudgetPeriod,
  BudgetSubjectType,
  CreateBudgetRequest,
} from "@/lib/api/types";
import { getChannelTypeDisplayName } from "@/lib/app-channels";

type SubjectChoice = { kind: "app" } | { kind: "channel"; channelId: string };

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

function getBudgetCurrencyDisplayName(currency: string): string {
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

const PERIOD_PRESET_TO_PERIOD: Record<Exclude<PeriodPreset, "none" | "custom">, BudgetPeriod> = {
  "1h": { type: "duration", seconds: 3_600 },
  "5h": { type: "duration", seconds: 5 * 3_600 },
  "24h": { type: "duration", seconds: 24 * 3_600 },
  "7d": { type: "duration", seconds: 7 * 24 * 3_600 },
  "30d": { type: "duration", seconds: 30 * 24 * 3_600 },
  calendar_month: { type: "calendar", unit: "month" },
};

function formatPeriod(period?: BudgetPeriod | null): string {
  if (!period) return "one-shot";
  if (period.type === "duration") {
    const sec = period.seconds;
    if (sec % 86_400 === 0) return `${sec / 86_400}d sliding`;
    if (sec % 3_600 === 0) return `${sec / 3_600}h sliding`;
    if (sec % 60 === 0) return `${sec / 60}m sliding`;
    return `${sec}s sliding`;
  }
  if (period.type === "rolling") return `${period.window} sliding`;
  return `calendar ${period.unit}`;
}

function subjectLabel(budget: Budget, app: App): string {
  if (budget.subject_type === "app") return "App-wide";
  if (budget.subject_type === "app_channel") {
    const ch = app.channels?.find((c) => c.id === budget.subject_id);
    return ch ? `Channel: ${ch.channel_type}` : `Channel ${budget.subject_id}`;
  }
  return budget.subject_type;
}

export function AppBudgetsCard({ app, readOnly }: { app: App; readOnly: boolean }) {
  const queryClient = useQueryClient();
  const queryKey = ["app-budgets", app.id];

  const { data: budgets = [], isLoading } = useQuery({
    queryKey,
    queryFn: () => listAppBudgets(app.id, { includeChannels: true }),
  });

  const [adding, setAdding] = useState(false);

  const createMutation = useMutation({
    mutationFn: (input: CreateBudgetRequest) => createBudget(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey });
      setAdding(false);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (budgetId: string) => deleteBudget(budgetId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey }),
  });

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="flex items-center gap-2">
          Budgets
          <Badge variant="outline">experimental</Badge>
        </CardTitle>
        {!readOnly && !adding && (
          <Button variant="outline" size="sm" onClick={() => setAdding(true)}>
            <Plus className="w-3 h-3 mr-1" />
            Add budget
          </Button>
        )}
      </CardHeader>
      <CardContent className="space-y-3">
        {isLoading && <p className="text-sm text-muted-foreground">Loading budgets…</p>}

        {!isLoading && budgets.length === 0 && !adding && (
          <p className="text-sm text-muted-foreground">
            No budgets attached. Add one to cap LLM spend on this app or any of its channels.
          </p>
        )}

        {budgets.map((budget) => (
          <div
            key={budget.id}
            className="flex items-start justify-between gap-3 rounded-md border p-3"
          >
            <div className="space-y-1">
              <div className="flex items-center gap-2 text-sm">
                <Badge variant="secondary">{subjectLabel(budget, app)}</Badge>
                <Badge variant="outline">{budget.currency}</Badge>
                <Badge variant={budget.status === "active" ? "outline" : "secondary"}>
                  {budget.status}
                </Badge>
              </div>
              <div className="text-xs text-muted-foreground">
                Limit {budget.limit} • Balance {budget.balance.toFixed(2)} •{" "}
                {formatPeriod(budget.period)}
                {budget.period_started_at && (
                  <>
                    {" • started "}
                    {new Date(budget.period_started_at).toLocaleString()}
                  </>
                )}
              </div>
            </div>
            {!readOnly && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => deleteMutation.mutate(budget.id)}
                disabled={deleteMutation.isPending}
              >
                <Trash2 className="w-3 h-3" />
              </Button>
            )}
          </div>
        ))}

        {createMutation.error instanceof Error && (
          <p className="text-sm text-destructive">{createMutation.error.message}</p>
        )}

        {adding && (
          <NewBudgetForm
            app={app}
            submitting={createMutation.isPending}
            onCancel={() => setAdding(false)}
            onSubmit={(input) => createMutation.mutate(input)}
          />
        )}
      </CardContent>
    </Card>
  );
}

function NewBudgetForm({
  app,
  submitting,
  onCancel,
  onSubmit,
}: {
  app: App;
  submitting: boolean;
  onCancel: () => void;
  onSubmit: (input: CreateBudgetRequest) => void;
}) {
  const [subject, setSubject] = useState<SubjectChoice>({ kind: "app" });
  const [currency, setCurrency] = useState<string>("usd");
  const [limit, setLimit] = useState<string>("10");
  const [softLimit, setSoftLimit] = useState<string>("");
  const [periodPreset, setPeriodPreset] = useState<PeriodPreset>("calendar_month");
  const [customPeriod, setCustomPeriod] = useState<string>(
    JSON.stringify({ type: "duration", seconds: 3600 }, null, 2),
  );
  const [error, setError] = useState<string | null>(null);

  const channels = app.channels ?? [];

  function buildPeriod(): BudgetPeriod | null | undefined {
    if (periodPreset === "none") return null;
    if (periodPreset === "custom") {
      try {
        const parsed = JSON.parse(customPeriod) as BudgetPeriod;
        return parsed;
      } catch {
        throw new Error("Custom period is not valid JSON");
      }
    }
    return PERIOD_PRESET_TO_PERIOD[periodPreset];
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const limitNum = Number.parseFloat(limit);
    if (!Number.isFinite(limitNum) || limitNum <= 0) {
      setError("Limit must be a positive number");
      return;
    }
    let softNum: number | undefined;
    if (softLimit.trim().length > 0) {
      const parsed = Number.parseFloat(softLimit);
      if (!Number.isFinite(parsed) || parsed <= 0 || parsed > limitNum) {
        setError("Soft limit must be between 0 and limit");
        return;
      }
      softNum = parsed;
    }
    let period: BudgetPeriod | null | undefined;
    try {
      period = buildPeriod();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Invalid period");
      return;
    }

    const subjectType: BudgetSubjectType = subject.kind === "app" ? "app" : "app_channel";
    const subjectId = subject.kind === "app" ? app.id : subject.channelId;

    onSubmit({
      subject_type: subjectType,
      subject_id: subjectId,
      currency,
      limit: limitNum,
      soft_limit: softNum ?? null,
      period: period ?? null,
    });
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3 rounded-md border border-dashed p-3">
      <div className="grid gap-3 md:grid-cols-2">
        <div className="space-y-1">
          <Label>Scope</Label>
          <Select
            value={subject.kind === "app" ? "app" : `ch:${subject.channelId}`}
            onValueChange={(value) =>
              setSubject(
                value === "app" ? { kind: "app" } : { kind: "channel", channelId: value.slice(3) },
              )
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="app">App-wide</SelectItem>
              {channels.map((channel: AppChannel) => (
                <SelectItem key={channel.id} value={`ch:${channel.id}`}>
                  Channel: {getChannelTypeDisplayName(channel.channel_type)}
                  {channels.length > 1 ? ` (${channel.id.slice(-6)})` : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-1">
          <Label>Currency</Label>
          <Select value={currency} onValueChange={setCurrency}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="usd">{getBudgetCurrencyDisplayName("usd")}</SelectItem>
              <SelectItem value="tokens">{getBudgetCurrencyDisplayName("tokens")}</SelectItem>
              <SelectItem value="credits">{getBudgetCurrencyDisplayName("credits")}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-1">
          <Label>Limit</Label>
          <Input
            type="number"
            step="0.01"
            min="0"
            value={limit}
            onChange={(e) => setLimit(e.target.value)}
            required
          />
        </div>

        <div className="space-y-1">
          <Label>Soft limit (optional)</Label>
          <Input
            type="number"
            step="0.01"
            min="0"
            value={softLimit}
            onChange={(e) => setSoftLimit(e.target.value)}
            placeholder="pause when crossed"
          />
        </div>

        <div className="space-y-1 md:col-span-2">
          <Label>Period</Label>
          <Select
            value={periodPreset}
            onValueChange={(value) => setPeriodPreset(value as PeriodPreset)}
          >
            <SelectTrigger>
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

        {periodPreset === "custom" && (
          <div className="space-y-1 md:col-span-2">
            <Label>Custom period JSON</Label>
            <Textarea
              value={customPeriod}
              onChange={(e) => setCustomPeriod(e.target.value)}
              rows={4}
              spellCheck={false}
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">
              Examples: <code>{'{"type":"duration","seconds":18000}'}</code> for 5 hours,{" "}
              <code>{'{"type":"calendar","unit":"week"}'}</code> for weekly.
            </p>
          </div>
        )}
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      <div className="flex gap-2">
        <Button type="submit" disabled={submitting}>
          {submitting ? "Creating…" : "Create budget"}
        </Button>
        <Button type="button" variant="outline" onClick={onCancel} disabled={submitting}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
