import { render, screen, waitFor } from "@testing-library/react";
import { AgentPreview } from "@/components/agents/agent-preview";
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

jest.mock("@/hooks/use-agents", () => ({
  usePreviewAgent: () => mockUsePreviewAgent(),
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
      (
        _req: unknown,
        callbacks?: { onSuccess?: (d: AgentPreviewResponse) => void },
      ) => {
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

    render(
      <AgentPreview
        systemPrompt="hi"
        capabilities={[]}
        initialFiles={[]}
      />,
    );

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
    mockUsePreviewAgent.mockReturnValue(
      makeMutationStub({ error: new Error("boom") }),
    );

    render(
      <AgentPreview
        systemPrompt="hi"
        capabilities={[]}
        initialFiles={[]}
      />,
    );

    expect(screen.getByText("Preview Error")).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  // Regression for "TypeError: t is not iterable" in the agent preview tab when an
  // older agent record arrives without the `initial_files` field.
  it.each([undefined, null])(
    "does not crash when agent.initial_files is %p",
    (initialFiles) => {
      mockUsePreviewAgent.mockReturnValue(makeMutationStub({ data: sampleResponse }));

      render(
        <AgentPreview
          systemPrompt="hi"
          capabilities={[]}
          initialFiles={initialFiles as never}
        />,
      );

      expect(screen.getByText("Initial Files")).toBeInTheDocument();
      expect(screen.getByText("No initial files configured.")).toBeInTheDocument();
    },
  );
});
