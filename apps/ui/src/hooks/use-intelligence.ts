"use client";

/**
 * Whether this org can actually run a chat turn, and whether the current user
 * can do anything about it.
 *
 * Decision: readiness is derived from the model list rather than the provider
 * list. A provider row alone proves nothing — a keyed provider whose models were
 * never discovered, or whose models are all disabled, resolves to no model and
 * the turn fails. `healthy` is the server's own derivation (provider active and
 * credentialed), so this stays in step with `get_default_model`, which also
 * fails closed on a disabled model or an inactive provider.
 *
 * Every role may read models (`org:providers:view`), so the check itself is not
 * gated; only the fix-it link is, on `provider.manage`.
 */

import { useModels, useProvidersConfig } from "@/hooks/use-providers";
import { isChatModel } from "@/lib/model-capabilities";

export interface IntelligenceStatus {
  /** Still resolving — render nothing rather than a false alarm. */
  isLoading: boolean;
  /** At least one enabled, healthy chat model exists. */
  available: boolean;
  /** The caller may configure providers and models. */
  canManage: boolean;
}

export function useIntelligenceStatus(): IntelligenceStatus {
  const { data: models, isLoading: modelsLoading, isError: modelsError } = useModels();
  const { data: config, isLoading: configLoading } = useProvidersConfig();

  // A failed models read is not evidence of a misconfigured org, so treat it as
  // "nothing to say" and leave the chat UI alone.
  const available =
    modelsError || !models
      ? true
      : models.some((model) => model.enabled && model.healthy && isChatModel(model));

  return {
    isLoading: modelsLoading || configLoading,
    available,
    canManage: config?.policies?.["provider.manage"] ?? false,
  };
}
