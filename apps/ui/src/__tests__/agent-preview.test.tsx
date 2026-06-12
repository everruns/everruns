import { render, screen, waitFor } from "@testing-library/react";
import { AgentPreview, applyByteSpanReplacement } from "@/components/agents/agent-preview";
import type { AgentPreviewResponse } from "@/lib/api/types";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => (
    <pre data-testid="streamdown-mock">{children}</pre>
  ),
}));

jest.mock("@streamdown/code", () => ({
  code: {},
}));

const mockUsePreviewAgent = jest.fn();
const mockUseAnalyzeAgent = jest.fn(() => ({
  mutate: jest.fn(),
  isPending: false,
  error: null,
}));

jest.mock("@/hooks/use-agents", () => ({
  usePreviewAgent: () => mockUsePreviewAgent(),
  useAnalyzeAgent: () => mockUseAnalyzeAgent(),
}));

function makeMutationStub(opts: {
  data?: AgentPreviewResponse | null;
  isPending?: boolean;
  error?: Error | null;
}) {
  // The component calls `previewMutation.mutate(req, { onSuccess })` from useEffect.
  // We invoke onSuccess synchronously when data is supplied to drive the success path.
  return {
    mutate: jest.fn(
      (_req: unknown, callbacks?: { onSuccess?: (d: AgentPreviewResponse) => void }) => {
        if (opts.data && callbacks?.onSuccess) callbacks.onSuccess(opts.data);
      },
    ),
    isPending: opts.isPending ?? false,
    error: opts.error ?? null,
  };
}

const sampleResponse: AgentPreviewResponse = {
  system_prompt: "## System prompt\n\nYou are helpful.",
  tools: [
    {
      type: "builtin",
      name: "search_web",
      description: "Search the web",
      parameters: { type: "object", properties: {} },
    },
  ],
};

describe("AgentPreview", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("shows skeletons while the preview request is pending", () => {
    mockUsePreviewAgent.mockReturnValue(makeMutationStub({ isPending: true }));

    render(<AgentPreview systemPrompt="hi" capabilities={[]} initialFiles={[]} />);

    expect(screen.queryByText("Full System Prompt")).not.toBeInTheDocument();
  });

  it("renders system prompt, tools, and initial files preview on success", async () => {
    mockUsePreviewAgent.mockReturnValue(makeMutationStub({ data: sampleResponse }));

    render(
      <AgentPreview
        systemPrompt="base prompt"
        capabilities={[]}
        initialFiles={[
          {
            path: "/notes.txt",
            content: "hello",
            encoding: "text",
            is_readonly: false,
          },
        ]}
      />,
    );

    await waitFor(() => expect(screen.getByText("Full System Prompt")).toBeInTheDocument());
    expect(screen.getByText(/You are helpful\./)).toBeInTheDocument();
    expect(screen.getByText("Available Tools")).toBeInTheDocument();
    expect(screen.getByText("search_web")).toBeInTheDocument();
    // The InitialFilesPreview header is "Initial Files".
    expect(screen.getByText("Initial Files")).toBeInTheDocument();
    expect(screen.getByText("hello")).toBeInTheDocument();
  });

  it("shows the error card when the preview mutation fails", () => {
    mockUsePreviewAgent.mockReturnValue(makeMutationStub({ error: new Error("boom") }));

    render(<AgentPreview systemPrompt="hi" capabilities={[]} initialFiles={[]} />);

    expect(screen.getByText("Preview Error")).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  // Regression for "TypeError: t is not iterable" in the agent preview tab when an
  // older agent record arrives without the `initial_files` field.
  it.each([undefined, null])("does not crash when agent.initial_files is %p", (initialFiles) => {
    mockUsePreviewAgent.mockReturnValue(makeMutationStub({ data: sampleResponse }));

    render(<AgentPreview systemPrompt="hi" capabilities={[]} initialFiles={initialFiles} />);

    expect(screen.getByText("Initial Files")).toBeInTheDocument();
    expect(screen.getByText("No initial files configured.")).toBeInTheDocument();
  });
});

describe("applyByteSpanReplacement", () => {
  it("replaces an ascii byte span", () => {
    expect(applyByteSpanReplacement("Use `search_web` now.", 4, 16, "`web_fetch`")).toBe(
      "Use `web_fetch` now.",
    );
  });

  it("handles multibyte text before the span", () => {
    // "héllo " is 7 bytes (é = 2 bytes); span covers "bad".
    expect(applyByteSpanReplacement("héllo bad end", 7, 10, "good")).toBe("héllo good end");
  });
});

describe("AgentPreview findings", () => {
  it("renders findings from the preview response", async () => {
    mockUsePreviewAgent.mockReturnValue(
      makeMutationStub({
        data: {
          ...sampleResponse,
          findings: [
            {
              rule_id: "prompt.template_variables",
              severity: "warning",
              category: "structure",
              message: "Template placeholder found.",
              source: "builtin",
            },
          ],
        },
      }),
    );
    render(<AgentPreview systemPrompt="hi {{x}}" capabilities={[]} initialFiles={[]} />);
    await waitFor(() => {
      expect(screen.getByText("Template placeholder found.")).toBeInTheDocument();
      expect(screen.getByText("prompt.template_variables")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /analyze/i })).toBeInTheDocument();
    });
  });

  it("shows a clean state with the analyze button when no findings", async () => {
    mockUsePreviewAgent.mockReturnValue(
      makeMutationStub({ data: { ...sampleResponse, findings: [] } }),
    );
    render(<AgentPreview systemPrompt="hi" capabilities={[]} initialFiles={[]} />);
    await waitFor(() => {
      expect(screen.getByText(/no issues found by built-in rules/i)).toBeInTheDocument();
    });
  });
});
