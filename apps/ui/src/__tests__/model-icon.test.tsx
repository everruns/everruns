import { render } from "@testing-library/react";
import { ModelIcon } from "@/components/models/model-icon";
import type { LlmModelWithProvider } from "@/lib/api/types";

function model(
  model_id: string,
  family: string | undefined,
  provider_type: LlmModelWithProvider["provider_type"] = "openai_completions",
) {
  return { model_id, provider_type, profile: family ? { family } : null };
}

describe("ModelIcon vendor selection", () => {
  it("renders a per-vendor icon for OpenAI-compatible third-party models", () => {
    const cases: Array<[ReturnType<typeof model>, string]> = [
      [model("nemotron-3-super-120b-a12b", "nemotron-3-super"), "NVIDIA"],
      [model("nvidia/nemotron-3-super-120b-a12b", undefined), "NVIDIA"],
      [model("qwen/qwen3.7-max", "qwen3.7-max"), "Qwen"],
      [model("MiniMax-M3", "minimax-m3"), "MiniMax"],
      [model("kimi-k2-thinking", "kimi-k2-thinking"), "Moonshot"],
      [model("x-ai/grok-4.3", "grok-4.3"), "xAI"],
      [model("microsoft/MAI-1-preview", "mai-1-preview"), "Microsoft"],
    ];
    for (const [m, label] of cases) {
      const { getByTitle, unmount } = render(<ModelIcon model={m} />);
      expect(getByTitle(label)).toBeInTheDocument();
      unmount();
    }
  });

  it("falls back to the provider icon when no vendor is recognized", () => {
    // Claude Opus 4.8 and Gemini map to their first-party provider icons.
    const opus = render(
      <ModelIcon model={model("claude-opus-4-8", "claude-opus-4-8", "anthropic")} />,
    );
    expect(opus.getByTitle("Anthropic")).toBeInTheDocument();
    opus.unmount();

    const gemini = render(
      <ModelIcon model={model("gemini-3.1-pro-preview", "gemini-3.1-pro-preview", "gemini")} />,
    );
    expect(gemini.getByTitle("Google Gemini")).toBeInTheDocument();
    gemini.unmount();
  });
});
