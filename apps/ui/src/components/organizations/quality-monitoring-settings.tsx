"use client";

import { useState } from "react";
import Link from "next/link";
import { z } from "zod";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useUpdateOrganization } from "@/hooks/use-organizations";
import { usePageSettingsDraft } from "@/components/chat-mode/use-page-settings-draft";
import { ModelPicker } from "@/components/models/model-picker";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  getQualityMonitoring,
  getQualityMonitoringScores,
  reviewQualityMonitoringScore,
  type QualityMonitoringPolicy,
} from "@/lib/api/quality-monitoring";
import type { Organization } from "@/lib/api/types";

const defaults: QualityMonitoringPolicy = {
  enabled: false,
  sampling_rate: 0.02,
  model_id: null,
  daily_budget_usd: 0,
  excluded_tags: [],
};
const schema = z.strictObject({
  enabled: z.boolean(),
  sampling_rate: z.number().min(0).max(1),
  model_id: z.string().nullable(),
  daily_budget_usd: z.number().min(0).max(100),
  excluded_tags: z.array(z.string().min(1).max(128)).max(32),
});

export function QualityMonitoringSettings({
  organization,
  canManage,
}: {
  organization: Organization;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const update = useUpdateOrganization();
  const queryClient = useQueryClient();
  const stored = { ...defaults, ...organization.quality_monitoring };
  const [form, setForm] = useState(stored);
  const [baseline, setBaseline] = useState(stored);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [reviewing, setReviewing] = useState<string | null>(null);
  const [reviewError, setReviewError] = useState<string | null>(null);
  const report = useQuery({
    queryKey: ["quality-monitoring", organization.id],
    queryFn: () => getQualityMonitoring(organization.id),
    refetchInterval: 30000,
  });
  const scores = useQuery({
    queryKey: ["quality-monitoring-scores", organization.id],
    queryFn: () => getQualityMonitoringScores(organization.id),
    enabled: canManage,
    refetchInterval: 30000,
  });
  const money = (value: number) =>
    new Intl.NumberFormat(displayLocale, {
      style: "currency",
      currency: "USD",
      minimumFractionDigits: 4,
    }).format(value);

  async function save(policy: QualityMonitoringPolicy) {
    try {
      setError(null);
      const validated = schema.parse(policy);
      if (validated.enabled && validated.daily_budget_usd > 0 && !validated.model_id)
        throw new Error(ui("Select a monitoring model before enabling paid checks."));
      const response = await update.mutateAsync({ quality_monitoring: validated });
      const next = { ...defaults, ...response.quality_monitoring };
      setForm(next);
      setBaseline(next);
      setSaved(true);
      await queryClient.invalidateQueries({ queryKey: ["quality-monitoring", organization.id] });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not save."));
      throw cause;
    }
  }

  const draft = usePageSettingsDraft({
    values: baseline,
    schema,
    entityType: "organization",
    entityId: organization.id,
    formId: "quality-monitoring",
    label: ui("Compaction quality monitoring"),
    writable: canManage && !update.isPending,
    save,
  });
  const values = draft.adopted ? draft.values : form;
  const dirty = JSON.stringify(values) !== JSON.stringify(baseline);
  const readOnly = !canManage || update.isPending;
  function change(patch: Partial<QualityMonitoringPolicy>) {
    if (readOnly) return;
    const next = { ...values, ...patch };
    if (draft.adopted) draft.setValues(next);
    else setForm(next);
    setSaved(false);
    setError(null);
  }
  async function review(score: string, agrees: boolean) {
    setReviewing(score);
    setReviewError(null);
    try {
      await reviewQualityMonitoringScore(organization.id, score, agrees);
      await queryClient.invalidateQueries({
        queryKey: ["quality-monitoring-scores", organization.id],
      });
    } catch {
      setReviewError(ui("Could not save assessment review."));
    } finally {
      setReviewing(null);
    }
  }

  return (
    <Card
      id="quality-monitoring"
      className="space-y-5 p-5"
      data-testid="quality-monitoring-settings"
    >
      <div className="space-y-2">
        <h2 className="font-medium">{ui("Compaction quality monitoring")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui(
            "Automatically check compaction in real sessions. Operational checks use no judge model; sampled continuity checks use the organization model selected below.",
          )}
        </p>
        <p className="text-xs text-muted-foreground">
          {ui(
            "Findings stay in this organization. Sampled excerpts are processed by its selected provider. Monitoring is read-only, and findings are retained for 30 days.",
          )}
        </p>
      </div>
      <div className="flex items-center justify-between gap-4">
        <Label htmlFor="quality-monitor-enabled">{ui("Enable automatic monitoring")}</Label>
        <Switch
          id="quality-monitor-enabled"
          checked={values.enabled}
          disabled={readOnly}
          onCheckedChange={(enabled) => change({ enabled })}
        />
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="quality-monitor-model">{ui("Monitoring model")}</Label>
          <ModelPicker
            id="quality-monitor-model"
            organizationDefault
            value={values.model_id ?? ""}
            disabled={readOnly}
            placeholder={ui("Mechanical checks only")}
            additionalOptions={[{ value: "none", label: ui("Mechanical checks only") }]}
            onChange={(model) => change({ model_id: model === "none" ? null : model })}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="quality-monitor-budget">{ui("Daily monitoring budget (USD)")}</Label>
          <Input
            id="quality-monitor-budget"
            type="number"
            min={0}
            max={100}
            step={0.1}
            value={values.daily_budget_usd}
            disabled={readOnly}
            onChange={(event) => change({ daily_budget_usd: Number(event.target.value) })}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="quality-monitor-sample">{ui("Continuity sample (%)")}</Label>
          <Input
            id="quality-monitor-sample"
            type="number"
            min={0}
            max={100}
            step={1}
            value={Number((values.sampling_rate * 100).toFixed(2))}
            disabled={readOnly}
            onChange={(event) => change({ sampling_rate: Number(event.target.value) / 100 })}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="quality-monitor-exclude">{ui("Excluded session tags")}</Label>
          <Input
            id="quality-monitor-exclude"
            value={values.excluded_tags.join(", ")}
            disabled={readOnly}
            onChange={(event) =>
              change({
                excluded_tags: event.target.value
                  .split(",")
                  .map((tag) => tag.trim())
                  .filter(Boolean),
              })
            }
          />
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        {ui(
          "A zero budget enables mechanical checks only. The daily UTC cap includes conservative retry reservations; uncertain calls keep their allowance for that day. Disabling monitoring stops new assessments, including queued work.",
        )}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {draft.adopted
        ? draft.controls
        : canManage && (
            <div className="flex items-center gap-3">
              <Button
                disabled={readOnly || !dirty}
                onClick={() => void save(values).catch(() => {})}
              >
                {update.isPending ? ui("Saving…") : ui("Save")}
              </Button>
              {saved && (
                <span role="status" className="text-sm text-success">
                  {ui("Saved")}
                </span>
              )}
            </div>
          )}
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          {ui("Only organization owners and admins can change these settings.")}
        </p>
      )}
      <div className="space-y-2 border-t pt-4">
        <h3 className="text-sm font-medium">{ui("Monitoring coverage · 7 days")}</h3>
        {report.isError ? (
          <p role="alert" className="text-sm text-destructive">
            {ui("Could not load monitoring coverage.")}
          </p>
        ) : report.isPending ? (
          <p className="text-sm text-muted-foreground">{ui("Loading…")}</p>
        ) : (
          <>
            <p className="text-xs text-muted-foreground">
              {ui("Today: {reserved} reserved allowance · {reported} provider-reported cost", {
                reserved: money(report.data.today.reserved_usd),
                reported: money(report.data.today.reported_usd),
              })}
            </p>
            {report.data.checks.length === 0 && (
              <p className="text-sm text-muted-foreground">
                {ui("No compaction assessments yet.")}
              </p>
            )}
            {report.data.checks.map((check) => (
              <div key={check.check} className="text-sm">
                <span className="font-medium">
                  {check.check.includes("continuity")
                    ? ui("Continuity checks")
                    : ui("Operational checks")}
                </span>
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "Passed {passed} · Failed {failed} · Insufficient evidence {unknown} · Pending {pending}",
                    {
                      passed: check.passed,
                      failed: check.failed,
                      unknown: check.insufficient_evidence,
                      pending: check.pending,
                    },
                  )}
                </p>
              </div>
            ))}
          </>
        )}
      </div>
      {canManage && (
        <div className="space-y-3 border-t pt-4">
          <h3 className="text-sm font-medium">{ui("Recent findings")}</h3>
          {reviewError && (
            <p role="alert" className="text-sm text-destructive">
              {reviewError}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            {ui(
              "An operational pass confirms the reported execution step. Continuity verdicts apply to sampled evidence and do not prove complete memory preservation or causality.",
            )}
          </p>
          {scores.isError && (
            <p role="alert" className="text-sm text-destructive">
              {ui("Could not load monitoring findings.")}
            </p>
          )}
          {scores.data?.slice(0, 10).map((score) => (
            <div key={score.id} className="space-y-1 border p-3 text-sm">
              <div className="flex flex-wrap justify-between gap-2">
                <span className="font-medium">
                  {score.scorer_key.includes("continuity")
                    ? ui("Continuity checks")
                    : ui("Operational checks")}
                </span>
                <span>
                  {score.status !== "completed"
                    ? score.status === "pending" || score.status === "scoring"
                      ? ui("Pending")
                      : ui("Insufficient evidence")
                    : score.pass === true
                      ? ui("Passed")
                      : score.pass === false
                        ? ui("Failed")
                        : ui("Insufficient evidence")}
                </span>
              </div>
              {score.reason && <p className="text-xs text-muted-foreground">{score.reason}</p>}
              <Link
                className="text-xs text-primary hover:underline"
                href={`/sessions/${score.session_id}/trace`}
              >
                {ui("Open trace")}
              </Link>
              {score.status === "completed" && (
                <div className="flex flex-wrap gap-2 pt-1">
                  <Button
                    size="sm"
                    variant={score.review?.agrees === true ? "secondary" : "outline"}
                    aria-pressed={score.review?.agrees === true}
                    disabled={reviewing !== null}
                    onClick={() => void review(score.id, true)}
                  >
                    {ui("Agree with assessment")}
                  </Button>
                  <Button
                    size="sm"
                    variant={score.review?.agrees === false ? "secondary" : "outline"}
                    aria-pressed={score.review?.agrees === false}
                    disabled={reviewing !== null}
                    onClick={() => void review(score.id, false)}
                  >
                    {ui("Disagree with assessment")}
                  </Button>
                  {score.review && (
                    <span className="self-center text-xs text-muted-foreground">
                      {ui("Review recorded")}
                    </span>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
