import { isChatModel, isEmbeddingModel } from "@/lib/model-capabilities";

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
