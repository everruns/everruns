import { render } from "@testing-library/react";
import { EmbeddingModelPicker } from "@/components/knowledge-indexes/embedding-model-picker";

const mockUseModels = jest.fn();

jest.mock("@/hooks/use-providers", () => ({
  useModels: () => mockUseModels(),
}));

describe("EmbeddingModelPicker", () => {
  afterEach(() => {
    jest.restoreAllMocks();
  });

  beforeEach(() => {
    mockUseModels.mockReturnValue({
      data: [
        {
          id: "model_001",
          display_name: "text-embedding-3-small",
          provider_name: "OpenAI",
          enabled: true,
          capabilities: ["embeddings"],
        },
      ],
      isLoading: false,
    });
  });

  it("remains controlled when an initially empty value is selected", () => {
    const consoleError = jest.spyOn(console, "error").mockImplementation(() => undefined);
    const onChange = jest.fn();
    const { rerender } = render(<EmbeddingModelPicker value="" onChange={onChange} />);

    rerender(<EmbeddingModelPicker value="model_001" onChange={onChange} />);

    expect(consoleError).not.toHaveBeenCalledWith(
      expect.stringContaining("changing the uncontrolled value state of Select to be controlled"),
    );
  });
});
