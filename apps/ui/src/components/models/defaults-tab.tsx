"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { z } from "zod";
import { usePageSettingsDraft } from "@/components/chat-mode/use-page-settings-draft";

// Organization defaults, one row per service, grouped by what they drive:
// Chat (the model agents and sessions run on), Decisions (the Jev decision
// model and who answers system checks), and Other services (provider-level
// defaults). The groups follow the two kinds of default below, so a row's
// meaning comes from its section rather than repeated row text.
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

const defaultsSchema = z.strictObject({
  default_model_id: z.string(),
  small_model_id: z.string(),
  system_decisions: z.enum(["deployment", "organization"]),
  realtime_provider_id: z.string(),
  embeddings_provider_id: z.string(),
  images_provider_id: z.string(),
  rerank_provider_id: z.string(),
});
const decisionSchema = z.strictObject({ decision_model_id: z.string() });

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
  const ui = useUiTranslations();
  const { data: org } = useOrganization();
  const updateOrg = useUpdateOrganization();
  const decisionDefault = useDecisionDefault();
  const [error, setError] = useState<string | null>(null);

  // The PATCH replaces the whole per-service map, and the mutation only
  // invalidates the org query. A local draft keeps the latest selections so
  // rapid edits patch from the freshest state, then re-syncs from the server.
  const [legacyServiceDraft, setServiceDraft] = useState<Record<string, string>>(
    () => org?.default_provider_per_service ?? {},
  );
  useEffect(() => {
    if (org?.default_provider_per_service) setServiceDraft(org.default_provider_per_service);
  }, [org?.default_provider_per_service]);

  const settings = usePageSettingsDraft({
    schema: defaultsSchema,
    entityType: "model_defaults",
    entityId: org?.id,
    label: ui("Model defaults"),
    formId: "organization_defaults",
    enabled: !!org,
    writable: canManage && !!org && !updateOrg.isPending,
    values: {
      default_model_id: org?.default_model_id ?? "",
      small_model_id: org?.small_model_id ?? "",
      system_decisions: org?.system_decisions ?? "deployment",
      realtime_provider_id: org?.default_provider_per_service?.realtime ?? "",
      embeddings_provider_id: org?.default_provider_per_service?.embeddings ?? "",
      images_provider_id: org?.default_provider_per_service?.images ?? "",
      rerank_provider_id: org?.default_provider_per_service?.rerank ?? "",
    },
    saveLabel: "Save model defaults",
    save: async (values) => {
      const services = { ...org?.default_provider_per_service };
      for (const service of PROVIDER_SERVICES) {
        const value = values[`${service.key}_provider_id` as keyof typeof values];
        if (value) services[service.key] = value;
        else delete services[service.key];
      }
      await updateOrg.mutateAsync({
        default_model_id: values.default_model_id || null,
        small_model_id: values.small_model_id || null,
        system_decisions: values.system_decisions,
        default_provider_per_service: services,
      });
    },
  });
  const decisions = usePageSettingsDraft({
    schema: decisionSchema,
    values: { decision_model_id: decisionDefault.data?.id ?? "" },
    entityType: "model_defaults",
    entityId: org?.id,
    label: ui("Decision model"),
    formId: "decision_model",
    enabled: !!org,
    writable: canManage && !decisionDefault.setDefault.isPending,
    saveLabel: "Save decision model",
    save: (values) => decisionDefault.setDefault.mutateAsync(values.decision_model_id || null),
  });
  const serviceDraft = settings.adopted
    ? Object.fromEntries(
        PROVIDER_SERVICES.map((service) => [
          service.key,
          settings.values[`${service.key}_provider_id` as keyof typeof settings.values],
        ]),
      )
    : legacyServiceDraft;

  // The current choice stays listed even when it can no longer be used, so the
  // picker shows what is set next to the warning instead of the placeholder.
  const currentModelId = settings.adopted
    ? settings.values.default_model_id
    : org?.default_model_id;
  const currentDecisionId = decisions.adopted
    ? decisions.values.decision_model_id
    : decisionDefault.data?.id;
  const chatOptions = useMemo(
    () =>
      models
        .filter(
          (model) =>
            model.id === currentModelId || (model.enabled && !model.stale && isChatModel(model)),
        )
        .sort(compareByRecency)
        .map((model) => ({ value: model.id, label: modelLabel(model) })),
    [models, currentModelId],
  );
  const currentSmallModelId = settings.adopted
    ? settings.values.small_model_id
    : org?.small_model_id;
  const smallModelOptions = useMemo(
    () =>
      models
        .filter(
          (model) =>
            model.id === currentSmallModelId ||
            (model.enabled &&
              !model.stale &&
              isChatModel(model) &&
              model.provider_type !== "chatgpt"),
        )
        .sort(compareByRecency)
        .map((model) => ({ value: model.id, label: modelLabel(model) })),
    [models, currentSmallModelId],
  );
  const decisionOptions = useMemo(
    () =>
      models
        .filter(
          (model) =>
            model.id === currentDecisionId ||
            (model.enabled &&
              !model.stale &&
              isDecisionModel(model) &&
              model.profile?.decisions?.calibrated === true),
        )
        .sort(compareByRecency)
        .map((model) => ({ value: model.id, label: modelLabel(model) })),
    [models, currentDecisionId],
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

  const decisionId = currentDecisionId ?? "";
  const systemDecisions = settings.adopted
    ? settings.values.system_decisions
    : (org?.system_decisions ?? "deployment");
  const busy = !canManage || !org || updateOrg.isPending || settings.pending;

  return (
    <div className="flex max-w-3xl flex-col gap-6">
      {settings.controls}
      {decisions.controls}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          {ui("Only organization admins change defaults.")}
        </p>
      )}
      <DefaultSection
        title={ui("Chat")}
        description={ui("Chat models for conversations and lightweight work.")}
      >
        <DefaultRow
          label={ui("Default model")}
          description={ui("Used when no model is set on the agent or session.")}
          problem={modelDefaultProblem(currentModelId, models)}
        >
          <Combobox
            options={chatOptions}
            value={currentModelId ?? ""}
            onValueChange={(value) =>
              settings.adopted
                ? settings.setValues({ ...settings.values, default_model_id: value })
                : void run(() => updateOrg.mutateAsync({ default_model_id: value || null }))
            }
            placeholder={PLATFORM_DEFAULT_LABEL}
            searchPlaceholder="Search chat models…"
            disabled={busy}
          />
        </DefaultRow>
        <DefaultRow
          label={ui("Small model")}
          description={ui("Shared model for lightweight work, including automatic session titles.")}
          problem={modelDefaultProblem(currentSmallModelId, models)}
        >
          <Combobox
            options={smallModelOptions}
            value={currentSmallModelId ?? ""}
            onValueChange={(value) =>
              settings.adopted
                ? settings.setValues({ ...settings.values, small_model_id: value })
                : void run(() => updateOrg.mutateAsync({ small_model_id: value || null }))
            }
            placeholder={ui("Not set · titles use session model")}
            searchPlaceholder="Search chat models…"
            disabled={busy}
          />
        </DefaultRow>
      </DefaultSection>
      <DefaultSection
        title={ui("Decisions")}
        description={ui(
          "Yes-or-no checks: the Jev capability, guardrails and the Slack relevance check.",
        )}
      >
        <DefaultRow
          label={ui("Default decision model")}
          description={ui(
            "Used by the Jev capability when no model is bound. Calibrated models only.",
          )}
          problem={
            decisionDefault.data ? modelDefaultProblem(decisionDefault.data.id, models) : null
          }
        >
          <Combobox
            options={decisionOptions}
            value={decisionId}
            onValueChange={(value) =>
              decisions.adopted
                ? decisions.setValues({ decision_model_id: value })
                : void run(() => decisionDefault.setDefault.mutateAsync(value || null))
            }
            placeholder={ui("No default")}
            searchPlaceholder="Search decision models…"
            disabled={!canManage || decisionDefault.setDefault.isPending || decisions.pending}
          />
        </DefaultRow>
        <DefaultRow
          label={ui("System decisions")}
          description={ui("Who answers guardrail jev checks and the Slack relevance check.")}
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
              settings.adopted
                ? settings.setValues({
                    ...settings.values,
                    system_decisions: value as SystemDecisionsSource,
                  })
                : void run(() =>
                    updateOrg.mutateAsync({ system_decisions: value as SystemDecisionsSource }),
                  )
            }
            disabled={busy}
          >
            <SelectTrigger aria-label={ui("System decisions")} className="w-full">
              <SelectValue>
                {systemDecisions === "organization"
                  ? ui("This organization's decision model")
                  : ui("Deployment default")}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="deployment">{ui("Deployment default")}</SelectItem>
              <SelectItem value="organization">
                {ui("This organization's decision model")}
              </SelectItem>
            </SelectContent>
          </Select>
        </DefaultRow>
      </DefaultSection>
      <DefaultSection
        title={ui("Other services")}
        description={ui(
          "Each defaults to a provider. A provider whose driver lacks the service is rejected when it runs.",
        )}
      >
        {PROVIDER_SERVICES.map((service) => (
          <DefaultRow
            key={service.key}
            label={service.label}
            description={service.description}
            problem={providerDefaultProblem(serviceDraft[service.key], providers)}
          >
            <Combobox
              options={providerOptions}
              value={serviceDraft[service.key] ?? ""}
              onValueChange={(providerId) => {
                if (settings.adopted) {
                  settings.setValues({
                    ...settings.values,
                    [`${service.key}_provider_id`]: providerId,
                  });
                  return;
                }
                const next = { ...serviceDraft };
                if (providerId) next[service.key] = providerId;
                else delete next[service.key];
                setServiceDraft(next);
                void run(() => updateOrg.mutateAsync({ default_provider_per_service: next }));
              }}
              placeholder={ui("No default")}
              searchPlaceholder="Search providers…"
              disabled={busy}
            />
          </DefaultRow>
        ))}
      </DefaultSection>
    </div>
  );
}

function DefaultSection({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  const id = `defaults-${title.toLowerCase().replace(/\s+/g, "-")}`;
  return (
    <section aria-labelledby={id} className="flex flex-col gap-2">
      <div>
        <h3 id={id} className="text-sm font-semibold">
          {title}
        </h3>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <div className="divide-y border bg-card">{children}</div>
    </section>
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
