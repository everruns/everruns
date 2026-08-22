/**
 * Decisions:
 * - Persist per-session model selection in localStorage so a chat reload keeps the user's override.
 * - Treat the server-provided session model as the fallback; empty selection means "Default".
 * - Keep reasoning validation close to model selection so unsupported effort values clear immediately.
 * - Track recently-used models globally (not per-session) so the picker can surface quick reselects.
 *   Recents are stored as bare IDs; they are resolved against the live model list at render time so
 *   deleted/disabled models fall out automatically instead of pointing at something that no longer exists.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ModelWithProvider, ReasoningEffort, Verbosity } from "@/lib/api/types";
import { isChatModel } from "@/lib/model-capabilities";

// How many recently-used models to remember and surface in the picker.
const RECENT_MODELS_LIMIT = 2;
const RECENT_MODELS_STORAGE_KEY = "everruns:chat:recent-models";

function readRecentModelIds(): string[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(RECENT_MODELS_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((id): id is string => typeof id === "string");
  } catch {
    return [];
  }
}

export function useChatModelSelection({
  agentId,
  sessionId,
  models,
  defaultModel,
  defaultModelLoading,
  modelsLoading,
  reasoningEffort,
  setReasoningEffort,
  verbosity,
  setVerbosity,
}: {
  agentId?: string;
  sessionId: string;
  models: ModelWithProvider[];
  defaultModel?: ModelWithProvider;
  defaultModelLoading: boolean;
  modelsLoading: boolean;
  reasoningEffort: ReasoningEffort | "";
  setReasoningEffort: (value: ReasoningEffort | "") => void;
  verbosity: Verbosity | "";
  setVerbosity: (value: Verbosity | "") => void;
}) {
  const [selectedModelId, setSelectedModelId] = useState("");
  const [recentModelIds, setRecentModelIds] = useState<string[]>([]);
  const hasUserSelectedModel = useRef(false);
  const storageKey = useMemo(
    () => `everruns:chat:model-selection:${agentId ?? "unknown"}:${sessionId}`,
    [agentId, sessionId],
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    const storedSelection = window.localStorage.getItem(storageKey);
    if (storedSelection !== null && !hasUserSelectedModel.current) {
      setSelectedModelId(storedSelection);
    }
  }, [storageKey]);

  useEffect(() => {
    const stored = readRecentModelIds();
    if (stored.length > 0) {
      setRecentModelIds(stored);
    }
  }, []);

  const selectedModel = useMemo(
    () =>
      models.find((model) => model.id === selectedModelId && model.enabled && isChatModel(model)),
    [models, selectedModelId],
  );

  // Resolve stored recent IDs against the live model list so anything deleted,
  // disabled, or otherwise missing simply drops out of the picker.
  const recentModels = useMemo(() => {
    const usable = new Map(
      models
        .filter((model) => model.enabled && isChatModel(model))
        .map((model) => [model.id, model]),
    );
    return recentModelIds
      .map((id) => usable.get(id))
      .filter((model): model is ModelWithProvider => Boolean(model))
      .slice(0, RECENT_MODELS_LIMIT);
  }, [models, recentModelIds]);

  const activeModel = selectedModel ?? defaultModel;
  const reasoningEffortConfig = activeModel?.profile?.reasoning_effort;
  const supportsReasoning = Boolean(
    activeModel?.profile?.reasoning && activeModel?.profile?.reasoning_effort,
  );

  useEffect(() => {
    if (!supportsReasoning) {
      setReasoningEffort("");
      return;
    }

    if (
      reasoningEffortConfig &&
      reasoningEffort &&
      !reasoningEffortConfig.values.some((effort) => effort.value === reasoningEffort)
    ) {
      setReasoningEffort("");
    }
  }, [supportsReasoning, reasoningEffortConfig, reasoningEffort, setReasoningEffort]);

  const getReasoningEffortName = (value: string): string => {
    const effort = reasoningEffortConfig?.values.find((item) => item.value === value);
    return effort?.name ?? value;
  };

  const defaultEffortName = reasoningEffortConfig?.default
    ? getReasoningEffortName(reasoningEffortConfig.default)
    : "Medium";

  const verbosityConfig = activeModel?.profile?.verbosity;
  const supportsVerbosity = Boolean(verbosityConfig);

  // Clear the verbosity override when it no longer applies: the active model has
  // no verbosity config, or the stored value isn't offered by this model.
  useEffect(() => {
    if (!supportsVerbosity) {
      setVerbosity("");
      return;
    }

    if (
      verbosityConfig &&
      verbosity &&
      !verbosityConfig.values.some((item) => item.value === verbosity)
    ) {
      setVerbosity("");
    }
  }, [supportsVerbosity, verbosityConfig, verbosity, setVerbosity]);

  const getVerbosityName = (value: string): string => {
    const item = verbosityConfig?.values.find((entry) => entry.value === value);
    return item?.name ?? value;
  };

  const defaultVerbosityName = verbosityConfig?.default
    ? getVerbosityName(verbosityConfig.default)
    : "Medium";

  const modelTriggerLabel = selectedModelId
    ? (selectedModel?.display_name ?? (modelsLoading ? "Loading model…" : "Unavailable model"))
    : defaultModel?.display_name
      ? `Default · ${defaultModel.display_name}`
      : defaultModelLoading
        ? "Default · Loading…"
        : "Default · Unavailable";
  const defaultModelOptionLabel = defaultModel?.display_name
    ? `Default · ${defaultModel.display_name}`
    : defaultModelLoading
      ? "Default · Loading…"
      : "Default · Unavailable";

  const handleModelChange = (value: string) => {
    hasUserSelectedModel.current = true;
    setSelectedModelId(value);
  };

  const recordRecentModel = useCallback((modelId: string) => {
    // "Default" (empty selection) is not a concrete model, so it is never recorded.
    if (typeof window === "undefined" || !modelId) return;
    setRecentModelIds((previous) => {
      const next = [modelId, ...previous.filter((id) => id !== modelId)].slice(
        0,
        RECENT_MODELS_LIMIT,
      );
      try {
        window.localStorage.setItem(RECENT_MODELS_STORAGE_KEY, JSON.stringify(next));
      } catch {
        // Storage may be unavailable (private mode, quota); recents are best-effort.
      }
      return next;
    });
  }, []);

  const persistSelection = () => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(storageKey, selectedModelId);
    recordRecentModel(selectedModelId);
  };

  return {
    selectedModelId,
    selectedModel,
    activeModel,
    recentModels,
    supportsReasoning,
    reasoningEffortConfig,
    defaultEffortName,
    supportsVerbosity,
    verbosityConfig,
    defaultVerbosityName,
    getVerbosityName,
    modelTriggerLabel,
    defaultModelOptionLabel,
    getReasoningEffortName,
    handleModelChange,
    persistSelection,
  };
}
