import { act, renderHook } from "@testing-library/react";
import { useChatModelSelection } from "@/hooks/use-chat-model-selection";
import type { ModelWithProvider } from "@/lib/api/types";

const RECENT_KEY = "everruns:chat:recent-models";

function makeModel(id: string, overrides: Partial<ModelWithProvider> = {}): ModelWithProvider {
  return {
    id,
    provider_id: "provider-1",
    model_id: id,
    display_name: id.toUpperCase(),
    capabilities: ["chat"],
    enabled: true,
    is_favorite: false,
    created_at: "2025-01-01T00:00:00Z",
    updated_at: "2025-01-01T00:00:00Z",
    provider_name: "OpenAI",
    provider_type: "openai",
    healthy: true,
    ...overrides,
  };
}

function renderSelection(models: ModelWithProvider[]) {
  return renderHook(() =>
    useChatModelSelection({
      agentId: "agent-1",
      sessionId: "session-1",
      models,
      defaultModel: undefined,
      reasoningEffort: "",
      setReasoningEffort: () => undefined,
      verbosity: "",
      setVerbosity: () => undefined,
    }),
  );
}

describe("useChatModelSelection recent models", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("records the selected model on persist, most recent first, capped at two", () => {
    const models = [makeModel("a"), makeModel("b"), makeModel("c")];
    const { result, rerender } = renderSelection(models);

    for (const id of ["a", "b", "c"]) {
      act(() => result.current.handleModelChange(id));
      act(() => result.current.persistSelection());
      rerender();
    }

    expect(result.current.recentModels.map((m) => m.id)).toEqual(["c", "b"]);
    expect(JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? "[]")).toEqual(["c", "b"]);
  });

  it("does not record the Default (empty) selection", () => {
    const { result, rerender } = renderSelection([makeModel("a")]);

    act(() => result.current.handleModelChange(""));
    act(() => result.current.persistSelection());
    rerender();

    expect(result.current.recentModels).toHaveLength(0);
    expect(window.localStorage.getItem(RECENT_KEY)).toBeNull();
  });

  it("drops recents that no longer exist or are disabled", () => {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify(["gone", "disabled", "ok"]));
    const models = [makeModel("disabled", { enabled: false }), makeModel("ok")];

    const { result } = renderSelection(models);

    // "gone" is missing entirely, "disabled" is not enabled — only "ok" survives.
    expect(result.current.recentModels.map((m) => m.id)).toEqual(["ok"]);
  });
});
