/**
 * Decisions:
 * - Persist per-session model selection in localStorage so a chat reload keeps the user's override.
 * - Treat the server-provided session model as the fallback; empty selection means "Default".
 * - Keep reasoning validation close to model selection so unsupported effort values clear immediately.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type { LlmModelWithProvider, ReasoningEffort } from "@/lib/api/types";

export function useChatModelSelection({
  agentId,
  sessionId,
  llmModels,
  defaultModel,
  reasoningEffort,
  setReasoningEffort,
}: {
  agentId?: string;
  sessionId: string;
  llmModels: LlmModelWithProvider[];
  defaultModel?: LlmModelWithProvider;
  reasoningEffort: ReasoningEffort | "";
  setReasoningEffort: (value: ReasoningEffort | "") => void;
}) {
  const [selectedModelId, setSelectedModelId] = useState("");
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

  const selectedModel = useMemo(
    () => llmModels.find((model) => model.id === selectedModelId),
    [llmModels, selectedModelId],
  );

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

  const modelTriggerLabel = selectedModel?.display_name ?? "Default";
  const defaultModelOptionLabel = defaultModel?.display_name
    ? `Default (${defaultModel.display_name})`
    : "Default";

  const handleModelChange = (value: string) => {
    hasUserSelectedModel.current = true;
    setSelectedModelId(value);
  };

  const persistSelection = () => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(storageKey, selectedModelId);
  };

  return {
    selectedModelId,
    selectedModel,
    activeModel,
    supportsReasoning,
    reasoningEffortConfig,
    defaultEffortName,
    modelTriggerLabel,
    defaultModelOptionLabel,
    getReasoningEffortName,
    handleModelChange,
    persistSelection,
  };
}
