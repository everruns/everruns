"use client";

// Organization defaults, one row per service.
//
// Chat and decisions default to a model; realtime, embeddings, images and
// rerank default to a provider (EVE-569: tier 2 of service-bound resolution,
// after an explicit per-consumer binding and before the single-active-provider
// fallback). Pickers are searchable because a catalog can hold hundreds of
// models, and each row says when its choice cannot currently be used.

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Combobox } from "@/components/ui/combobox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { useDecisionDefault } from "@/hooks/use-providers";
import { ApiError } from "@/lib/api/client";
import type { ModelWithProvider, Provider, SystemDecisionsSource } from "@/lib/api/types";
import { isChatModel, isDecisionModel } from "@/lib/model-capabilities";
import { compareByRecency, modelLabel } from "@/lib/model-selection";

const PROVIDER_SERVICES: { key: string; label: string; description: string }[] = [
  { key: "realtime", label: "Realtime (voice)", description: "Voice realtime sessions." },
  { key: "embeddings", label: "Embeddings", description: "Text embeddings." },
  { key: "images", label: "Images", description: "Image generation." },
  { key: "rerank", label: "Rerank", description: "Search-result reranking." },
];

export const PLATFORM_DEFAULT_LABEL = "Platform default · GPT-6 Luna";

/** Why a chosen default model cannot be used right now, or null when it can. */
export function modelDefaultProblem(
  id: string | null | undefined,
  models: ModelWithProvider[],
): string | null {
  if (!id) return null;
  const model = models.find((candidate) => candidate.id === id);
  if (!model) return "This model no longer exists. Choose another.";
  if (model.stale) return `${model.provider_name} no longer lists this model.`;
  if (!model.enabled) return "This model is disabled. Enable it or choose another.";
  if (!model.healthy) return `${model.provider_name} has no key or is disabled.`;
  return null;
}

/** Why a chosen default provider cannot be used right now, or null when it can. */
export function providerDefaultProblem(
  id: string | undefined,
  providers: Provider[],
): string | null {
  if (!id) return null;
  const provider = providers.find((candidate) => candidate.id === id);
  if (!provider) return "This provider no longer exists. Choose another.";
  if (provider.status === "disabled") return "This provider is disabled.";
  if (!provider.api_key_set) return "This provider has no key yet.";
  return null;
}

function errorText(error: unknown): string {
  if (error instanceof ApiError && error.message) return error.message;
  return "Could not save the default. Please try again.";
}

export function DefaultsTab({
  models,
  providers,
  canManage,
}: {
  models: ModelWithProvider[];
  providers: Provider[];
  canManage: boolean;
}) {
  const { data: org } = useOrganization();
  const updateOrg = useUpdateOrganization();
  const decisionDefault = useDecisionDefault();
  const [error, setError] = useState<string | null>(null);

  // The PATCH replaces the whole per-service map, and the mutation only
  // invalidates the org query. A local draft keeps the latest selections so
  // rapid edits patch from the freshest state, then re-syncs from the server.
  const [serviceDraft, setServiceDraft] = useState<Record<string, string>>(
    () => org?.default_provider_per_service ?? {},
  );
  useEffect(() => {
    if (org?.default_provider_per_service) setServiceDraft(org.default_provider_per_service);
  }, [org?.default_provider_per_service]);

  const chatOptions = useMemo(
    () =>
      models
        .filter((model) => model.enabled && !model.stale && isChatModel(model))
        .sort(compareByRecency)
        .map((model) => ({ value: model.id, label: modelLabel(model) })),
    [models],
  );
  const decisionOptions = useMemo(
    () =>
      models
        .filter(
          (model) =>
            model.enabled &&
            !model.stale &&
            isDecisionModel(model) &&
            model.profile?.decisions?.calibrated === true,
        )
        .sort(compareByRecency)
        .map((model) => ({ value: model.id, label: modelLabel(model) })),
    [models],
  );
  // Personal providers are skipped by shared defaults.
  const providerOptions = useMemo(
    () =>
      providers
        .filter((provider) => provider.provider_type !== "chatgpt")
        .map((provider) => ({ value: provider.id, label: provider.name })),
    [providers],
  );

  const run = async (action: () => Promise<unknown>) => {
    setError(null);
    try {
      await action();
    } catch (err) {
      setError(errorText(err));
    }
  };

  const decisionId = decisionDefault.data?.id ?? "";
  const systemDecisions = org?.system_decisions ?? "deployment";
  const busy = !canManage || !org || updateOrg.isPending;

  return (
    <div className="flex max-w-3xl flex-col gap-3">
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!canManage && (
        <p className="text-sm text-muted-foreground">Only organization admins change defaults.</p>
      )}
      <div className="divide-y border bg-card">
        <DefaultRow
          label="Default model"
          description="Used when no model is set on the agent or session."
          problem={modelDefaultProblem(org?.default_model_id, models)}
        >
          <Combobox
            options={chatOptions}
            value={org?.default_model_id ?? ""}
            onValueChange={(value) =>
              void run(() => updateOrg.mutateAsync({ default_model_id: value || null }))
            }
            placeholder={PLATFORM_DEFAULT_LABEL}
            searchPlaceholder="Search chat models…"
            disabled={busy}
          />
        </DefaultRow>
        <DefaultRow
          label="Default decision model"
          description="Used by the Jev capability when no model is bound. Calibrated models only."
          problem={
            decisionDefault.data ? modelDefaultProblem(decisionDefault.data.id, models) : null
          }
        >
          <Combobox
            options={decisionOptions}
            value={decisionId}
            onValueChange={(value) =>
              void run(() => decisionDefault.setDefault.mutateAsync(value || null))
            }
            placeholder="No default"
            searchPlaceholder="Search decision models…"
            disabled={!canManage || decisionDefault.setDefault.isPending}
          />
        </DefaultRow>
        <DefaultRow
          label="System decisions"
          description="Who answers guardrail jev checks and the Slack relevance check."
          problem={
            systemDecisions === "organization" && !decisionId
              ? "No decision model is set, so these checks get no answer: guardrails let messages through and Slack stays silent."
              : null
          }
          note={
            systemDecisions === "organization"
              ? "Uses this organization's decision model on its own account. There is no fallback to deployment keys."
              : undefined
          }
        >
          <Select
            value={systemDecisions}
            onValueChange={(value) =>
              void run(() =>
                updateOrg.mutateAsync({ system_decisions: value as SystemDecisionsSource }),
              )
            }
            disabled={busy}
          >
            <SelectTrigger aria-label="System decisions" className="w-full">
              <SelectValue>
                {systemDecisions === "organization"
                  ? "This organization's decision model"
                  : "Deployment default"}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="deployment">Deployment default</SelectItem>
              <SelectItem value="organization">This organization&apos;s decision model</SelectItem>
            </SelectContent>
          </Select>
        </DefaultRow>
        {PROVIDER_SERVICES.map((service) => (
          <DefaultRow
            key={service.key}
            label={service.label}
            description={`${service.description} Defaults to a provider; a provider whose driver lacks the service is rejected when it runs.`}
            problem={providerDefaultProblem(serviceDraft[service.key], providers)}
          >
            <Combobox
              options={providerOptions}
              value={serviceDraft[service.key] ?? ""}
              onValueChange={(providerId) => {
                const next = { ...serviceDraft };
                if (providerId) next[service.key] = providerId;
                else delete next[service.key];
                setServiceDraft(next);
                void run(() => updateOrg.mutateAsync({ default_provider_per_service: next }));
              }}
              placeholder="No default"
              searchPlaceholder="Search providers…"
              disabled={busy}
            />
          </DefaultRow>
        ))}
      </div>
    </div>
  );
}

function DefaultRow({
  label,
  description,
  problem,
  note,
  children,
}: {
  label: string;
  description: string;
  problem?: string | null;
  note?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-2 px-4 py-3.5">
      <div className="min-w-0 flex-[1_1_240px]">
        <div className="text-sm font-medium">{label}</div>
        <div className="text-xs text-muted-foreground">{description}</div>
      </div>
      <div className="flex min-w-[220px] flex-[0_1_300px] flex-col gap-1.5">
        <div aria-label={label}>{children}</div>
        {note && <p className="text-xs text-muted-foreground">{note}</p>}
        {problem && (
          <p role="status" className="text-xs text-warning">
            {problem}
          </p>
        )}
      </div>
    </div>
  );
}
