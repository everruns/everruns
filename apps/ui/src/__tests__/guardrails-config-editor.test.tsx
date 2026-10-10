import { fireEvent, render, screen } from "@testing-library/react";
import { CapabilitySettingsEditor } from "@/components/agents/capability-settings-editor";
import {
  appendPresetChecks,
  guardrailCheckProblems,
  GuardrailsConfigEditor,
  parseGuardrailsDraft,
  serializeGuardrailsDraft,
} from "@/components/agents/guardrails-config-editor";
import type { Capability, GuardrailExample } from "@/lib/api/types";

jest.mock("@/hooks/use-capabilities", () => ({
  useGuardrailExamples: jest.fn(),
}));

const { useGuardrailExamples } = jest.requireMock("@/hooks/use-capabilities") as {
  useGuardrailExamples: jest.Mock;
};

const secretPreset: GuardrailExample = {
  name: "secret-detection",
  display_name: "Secret & Credential Detection",
  description: "Blocks well-known credential formats.",
  tags: ["security"],
  check_types: ["regex"],
  stages: ["output"],
  data_egress: "none",
  config: {
    mode: "active",
    checks: [
      {
        id: "secret-output",
        stage: "output",
        type: "regex",
        on_fail: "block",
        patterns: ["AKIA[0-9A-Z]{16}"],
      },
    ],
  },
};

function chooseOption(name: string | RegExp) {
  const option = screen.getByRole("option", { name });
  fireEvent.pointerDown(option, { pointerType: "mouse", button: 0 });
  fireEvent.click(option);
}

beforeEach(() => {
  useGuardrailExamples.mockReturnValue({
    data: [secretPreset],
    isLoading: false,
    error: null,
  });
});

describe("guardrails config draft", () => {
  it("round-trips a Decision API judge and keeps advisory mode", () => {
    const draft = parseGuardrailsDraft({
      mode: "advisory",
      checks: [
        {
          id: "no-delete",
          stage: "tool_use",
          type: "llm_judge",
          prompt: "Block deletes.",
          engine: "jev",
          threshold: 70,
        },
      ],
    });

    expect(serializeGuardrailsDraft(draft)).toEqual({
      mode: "advisory",
      checks: [
        {
          id: "no-delete",
          stage: "tool_use",
          type: "llm_judge",
          prompt: "Block deletes.",
          engine: "jev",
          threshold: 70,
        },
      ],
    });
  });

  it("omits the default active mode and an empty check list", () => {
    expect(serializeGuardrailsDraft(parseGuardrailsDraft({}))).toEqual({});
  });

  it("writes an unrecognized check back unchanged", () => {
    const weird = { type: "future", stage: "output", custom: 1 };
    const draft = parseGuardrailsDraft({ checks: [weird] });
    expect(serializeGuardrailsDraft(draft)).toEqual({ checks: [weird] });
  });

  it("keeps fields the editor does not own", () => {
    const draft = parseGuardrailsDraft({
      checks: [{ stage: "output", type: "regex", patterns: ["x"], note: "keep" }],
    });
    expect(serializeGuardrailsDraft(draft)).toEqual({
      checks: [{ note: "keep", stage: "output", type: "regex", patterns: ["x"] }],
    });
  });

  it("appends a preset once when its check id is already present", () => {
    const preset = secretPreset.config;
    const once = appendPresetChecks(parseGuardrailsDraft({}), preset);
    const twice = appendPresetChecks(once, preset);
    expect(once.checks).toHaveLength(1);
    expect(twice.checks).toHaveLength(1);
    expect(twice.mode).toBe("active");
  });

  it("requires the fields a check type cannot run without", () => {
    const draft = parseGuardrailsDraft({
      checks: [
        { stage: "tool_use", type: "llm_judge", prompt: "  ", engine: "jev", threshold: 140 },
      ],
    });
    expect(guardrailCheckProblems(draft.checks[0])).toEqual([
      "Write the policy this check enforces.",
      "Threshold must be a whole number from 0 to 100.",
    ]);
  });

  it("flags a stage the check type cannot run on", () => {
    const draft = parseGuardrailsDraft({
      checks: [{ stage: "output", type: "llm_judge", prompt: "Block it." }],
    });
    expect(guardrailCheckProblems(draft.checks[0])).toContain(
      "This check cannot run at that stage.",
    );
  });
});

describe("GuardrailsConfigEditor", () => {
  it("shows Active and Advisory, not the raw mode values", () => {
    render(<GuardrailsConfigEditor config={{}} onChange={jest.fn()} />);

    const mode = screen.getByRole("combobox", { name: "Mode" });
    expect(mode).toHaveTextContent("Active");
    expect(mode).not.toHaveTextContent(/^active$/);

    fireEvent.click(mode);
    expect(screen.getByRole("option", { name: "Active" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Advisory" })).toBeInTheDocument();
  });

  it("switches to advisory mode", () => {
    const onChange = jest.fn();
    render(<GuardrailsConfigEditor config={{}} onChange={onChange} />);

    fireEvent.click(screen.getByRole("combobox", { name: "Mode" }));
    chooseOption("Advisory");

    expect(onChange).toHaveBeenCalledWith({ mode: "advisory" });
  });

  it("adds a preset's checks", () => {
    const onChange = jest.fn();
    render(<GuardrailsConfigEditor config={{}} onChange={onChange} />);

    fireEvent.click(screen.getByRole("button", { name: "Add preset" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Secret & Credential Detection/ }));

    expect(onChange).toHaveBeenCalledWith({
      checks: [
        {
          id: "secret-output",
          stage: "output",
          type: "regex",
          patterns: ["AKIA[0-9A-Z]{16}"],
        },
      ],
    });
    expect(screen.getByDisplayValue("AKIA[0-9A-Z]{16}")).toBeInTheDocument();
  });

  it("adds a Decision API policy judge", () => {
    const onChange = jest.fn();
    render(<GuardrailsConfigEditor config={{}} onChange={onChange} />);

    fireEvent.click(screen.getByRole("button", { name: "Add check" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Policy judge/ }));
    fireEvent.click(screen.getByRole("combobox", { name: "Answered by" }));
    chooseOption("Decision API");

    expect(onChange).toHaveBeenLastCalledWith({
      checks: [
        {
          stage: "tool_use",
          type: "llm_judge",
          prompt: "",
          engine: "jev",
        },
      ],
    });
  });

  it("renders an existing Decision API check with a capitalized engine label", () => {
    render(
      <GuardrailsConfigEditor
        config={{
          checks: [
            {
              stage: "output",
              type: "moderation",
              engine: "jev",
              threshold: 30,
            },
          ],
        }}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByRole("combobox", { name: "Answered by" })).toHaveTextContent("Decision API");
    expect(screen.getByLabelText("Threshold")).toHaveValue("30");
  });
});

describe("CapabilitySettingsEditor guardrails", () => {
  it("uses the guardrails editor instead of the generic schema form", () => {
    const capability = {
      id: "guardrails",
      name: "Guardrails",
      description: "Checks",
      status: "available",
      config_schema: {
        type: "object",
        properties: {
          mode: { type: "string", enum: ["active", "advisory"] },
        },
      },
    } as Capability;

    render(<CapabilitySettingsEditor capability={capability} config={{}} onChange={jest.fn()} />);

    expect(screen.getByRole("combobox", { name: "Mode" })).toHaveTextContent("Active");
    expect(screen.getByRole("button", { name: "Add preset" })).toBeInTheDocument();
    expect(screen.queryByText(/^active$/)).not.toBeInTheDocument();
  });
});
