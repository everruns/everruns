import { render } from "@testing-library/react";
import { ModelIcon } from "@/components/models/model-icon";
import type { ModelWithProvider, ModelVendor } from "@/lib/api/types";

function model(
  model_vendor: ModelVendor | undefined,
  provider_type: ModelWithProvider["provider_type"] = "openai_completions",
) {
  return { provider_type, model_vendor };
}

describe("ModelIcon vendor selection", () => {
  it("renders a per-vendor icon from the model_vendor tag", () => {
    const cases: Array<[ModelVendor, string]> = [
      ["nvidia", "NVIDIA"],
      ["qwen", "Qwen"],
      ["minimax", "MiniMax"],
      ["moonshot", "Moonshot"],
      ["xai", "xAI"],
      ["microsoft", "Microsoft"],
    ];
    for (const [vendor, label] of cases) {
      const { getByTitle, unmount } = render(<ModelIcon model={model(vendor)} />);
      expect(getByTitle(label)).toBeInTheDocument();
      unmount();
    }
  });

  it("falls back to the provider icon for first-party and unknown vendors", () => {
    // Claude Opus 4.8 (vendor anthropic) and Gemini (vendor google) use their
    // provider-type icons rather than a dedicated brand icon.
    const opus = render(<ModelIcon model={model("anthropic", "anthropic")} />);
    expect(opus.getByTitle("Anthropic")).toBeInTheDocument();
    opus.unmount();

    const gemini = render(<ModelIcon model={model("google", "gemini")} />);
    expect(gemini.getByTitle("Google Gemini")).toBeInTheDocument();
    gemini.unmount();

    const meta = render(<ModelIcon model={model("meta", "meta")} />);
    expect(meta.getByTitle("Meta Model API")).toBeInTheDocument();
    meta.unmount();

    // No vendor tag at all → provider icon.
    const bare = render(<ModelIcon model={model(undefined, "openai")} />);
    expect(bare.getByTitle("OpenAI (Responses)")).toBeInTheDocument();
    bare.unmount();
  });
});
