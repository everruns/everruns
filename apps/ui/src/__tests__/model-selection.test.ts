import {
  compareByRecency,
  modelLabel,
  recommendedModelIds,
  selectionChanges,
} from "@/lib/model-selection";
import type { ModelWithProvider } from "@/lib/api/types";

const model = (overrides: Partial<ModelWithProvider> & { id: string }): ModelWithProvider =>
  ({
    model_id: overrides.id,
    display_name: overrides.id,
    provider_id: "p1",
    provider_name: "OpenAI Prod",
    provider_type: "openai",
    healthy: true,
    enabled: false,
    capabilities: ["chat"],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as ModelWithProvider;

const profiled = (id: string, family: string, release_date: string, provider_id = "p1") =>
  model({ id, provider_id, profile: { family, release_date } } as never);

describe("recommendedModelIds", () => {
  it("picks the newest release of each family per provider and skips uncurated models", () => {
    const ids = recommendedModelIds([
      profiled("gpt-5", "gpt", "2025-08-01"),
      profiled("gpt-6", "gpt", "2026-06-01"),
      profiled("gpt-6-other", "gpt", "2025-01-01", "p2"),
      model({ id: "uncurated" }),
    ]);
    expect([...ids].sort()).toEqual(["gpt-6", "gpt-6-other"]);
  });
});

describe("selectionChanges", () => {
  it("returns only the models whose enabled state changes", () => {
    const models = [model({ id: "a", enabled: true }), model({ id: "b" }), model({ id: "c" })];
    expect(selectionChanges(models, new Set(["b", "c"]))).toEqual([
      { id: "a", enabled: false },
      { id: "b", enabled: true },
      { id: "c", enabled: true },
    ]);
    expect(selectionChanges(models, new Set(["a"]))).toEqual([]);
  });
});

describe("labels and ordering", () => {
  it("names the provider so two providers serving one model stay apart", () => {
    expect(modelLabel({ display_name: "GPT-6", provider_name: "OpenAI EU" })).toBe(
      "GPT-6 (OpenAI EU)",
    );
  });

  it("orders newest release first and undated last", () => {
    const sorted = [
      model({ id: "undated" }),
      profiled("old", "gpt", "2024-01-01"),
      profiled("new", "gpt", "2026-01-01"),
    ].sort(compareByRecency);
    expect(sorted.map((m) => m.id)).toEqual(["new", "old", "undated"]);
  });
});
