import { render, screen } from "@testing-library/react";
import { ModelEffortMenu } from "@/components/chat/model-effort-menu";
import type { Model, ReasoningEffortConfig } from "@/lib/api/types";

function makeModel(id: string): Model {
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
  };
}

const reasoningConfig: ReasoningEffortConfig = {
  default: "medium",
  values: [
    { value: "low", name: "Low" },
    { value: "high", name: "High" },
  ],
};

function renderMenu(overrides: Partial<React.ComponentProps<typeof ModelEffortMenu>> = {}) {
  return render(
    <ModelEffortMenu
      models={[makeModel("a"), makeModel("b")]}
      recentModels={[]}
      selectedModelId=""
      onModelChange={() => undefined}
      modelTriggerLabel="Default"
      defaultModelOptionLabel="Default"
      supportsReasoning={false}
      reasoningEffort=""
      reasoningEffortConfig={undefined}
      defaultEffortName="Medium"
      getReasoningEffortName={(value) => value}
      onReasoningEffortChange={() => undefined}
      supportsVerbosity={false}
      verbosity=""
      verbosityConfig={undefined}
      defaultVerbosityName="Medium"
      getVerbosityName={(value) => value}
      onVerbosityChange={() => undefined}
      {...overrides}
    />,
  );
}

describe("ModelEffortMenu trigger", () => {
  it("summarizes just the model when reasoning is unsupported", () => {
    renderMenu({ modelTriggerLabel: "GPT-5.4" });
    expect(screen.getByText("GPT-5.4")).not.toHaveClass("truncate");
    expect(screen.queryByText("Medium")).not.toBeInTheDocument();
  });

  it("appends the effective effort when reasoning is supported", () => {
    renderMenu({
      modelTriggerLabel: "GPT-5.4",
      supportsReasoning: true,
      reasoningEffortConfig: reasoningConfig,
      reasoningEffort: "high",
      getReasoningEffortName: (value) => (value === "high" ? "High" : value),
    });
    expect(screen.getByText("GPT-5.4")).toBeInTheDocument();
    expect(screen.getByText("High")).toBeInTheDocument();
  });

  it("falls back to the default effort name when no override is set", () => {
    renderMenu({
      supportsReasoning: true,
      reasoningEffortConfig: reasoningConfig,
      reasoningEffort: "",
      defaultEffortName: "Medium",
    });
    expect(screen.getByText("Medium")).toBeInTheDocument();
  });
});
