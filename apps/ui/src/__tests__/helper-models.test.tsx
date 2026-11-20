import { render, screen } from "@testing-library/react";
import {
  BUILTIN_HELPER_MODELS,
  helperModelErrors,
  helperSettings,
  targetModelId,
  withHelperSettings,
} from "@/lib/helper-models";
import {
  HelperModelChip,
  helperModelSpec,
} from "@/components/session/helper-model-chip";
import type { AgentCapabilityConfig } from "@/lib/api/types";
import type { ModelPreset } from "@/lib/api/model-presets";

jest.mock("@/hooks", () => ({
  useModels: () => ({ data: [{ id: "model_b", display_name: "GPT-6.2 Sol" }] }),
}));

const subagents = (
  config: Record<string, unknown> = {},
): AgentCapabilityConfig[] => [
  { ref: "subagents", config } as AgentCapabilityConfig,
];

describe("helper models", () => {
  it("defaults to the built-in keys with the agent picking", () => {
    const settings = helperSettings(subagents());
    expect(settings.enabled).toBe(true);
    expect(settings.models.map((m) => m.key)).toEqual([
      "same",
      "fast",
      "coding",
      "deep",
      "second_opinion",
    ]);
    expect(settings.pick).toBe("agent");
    expect(helperSettings([]).enabled).toBe(false);
  });

  it("stores the built-in list as no config and keeps other settings", () => {
    const stored = withHelperSettings(
      subagents({ max_subagent_depth: 2 }),
      BUILTIN_HELPER_MODELS,
      "agent",
    );
    expect(stored[0].config).toEqual({ max_subagent_depth: 2 });
    const custom = withHelperSettings(
      subagents(),
      [BUILTIN_HELPER_MODELS[0]],
      "auto",
    );
    expect(custom[0].config).toEqual({
      helper_models: [BUILTIN_HELPER_MODELS[0]],
      helper_model_pick: "auto",
    });
  });

  it("rejects what the server rejects", () => {
    expect(helperModelErrors([])).toHaveLength(1);
    expect(
      helperModelErrors([
        { key: "Bad Key", when: "x", target: { type: "same" } },
        { key: "a", when: "", target: { type: "same" } },
      ]),
    ).toHaveLength(2);
    expect(helperModelErrors(BUILTIN_HELPER_MODELS)).toEqual([]);
  });

  it("resolves a job through the agent preset, then the org default", () => {
    const presets = [
      {
        id: "provider:gemini",
        name: "Gemini",
        small_model_id: "model_g",
      } as ModelPreset,
    ];
    const org = () => "model_org";
    expect(
      targetModelId(
        { type: "job", job: "small" },
        presets,
        "provider:gemini",
        org,
      ),
    ).toBe("model_g");
    expect(
      targetModelId({ type: "job", job: "small" }, presets, undefined, org),
    ).toBe("model_org");
  });

  it("shows the helper key, model and who picked it on a subagent", () => {
    const spec = helperModelSpec({
      model: "second_opinion",
      model_id: "model_b",
      model_pick: "auto",
      model_pick_reason: "Verify work (82% sure)",
    });
    expect(spec).toBeDefined();
    render(<HelperModelChip spec={spec!} />);
    expect(screen.getByTestId("helper-model-chip")).toHaveTextContent(
      "second_opinion· GPT-6.2 Sol",
    );
    expect(
      screen.getByText("Auto: Verify work (82% sure)"),
    ).toBeInTheDocument();
    expect(helperModelSpec({ mode: "foreground" })).toBeUndefined();
  });
});
