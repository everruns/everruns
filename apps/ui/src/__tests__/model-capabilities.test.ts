import { isChatModel, isEmbeddingModel, matchesModelService } from "@/lib/model-capabilities";

describe("model capabilities", () => {
  it("keeps embedding models out of chat model selectors", () => {
    const embeddingModel = { capabilities: ["Embeddings"] };

    expect(isEmbeddingModel(embeddingModel)).toBe(true);
    expect(isChatModel(embeddingModel)).toBe(false);
  });

  it("treats existing untagged models as chat models", () => {
    expect(isChatModel({ capabilities: [] })).toBe(true);
  });
});

it("allows extraction capabilities without changing a model's chat identity", () => {
  const model = { service: "chat" as const, capabilities: ["extractions"] };
  expect(isChatModel(model)).toBe(true);
  expect(matchesModelService(model, "extractions")).toBe(true);
  expect(matchesModelService({ service: "chat", capabilities: [] }, "extractions")).toBe(false);
});
