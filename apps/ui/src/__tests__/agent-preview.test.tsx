import { act, render, screen, waitFor } from "@testing-library/react";
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
  features: ["file_system", "secrets", "key_value"],
};

describe("AgentPreview", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("previews the selected harness and refreshes when that selection changes", () => {
    const mutation = makeMutationStub({ data: sampleResponse });
    mockUsePreviewAgent.mockReturnValue(mutation);
    const props = { systemPrompt: "hi", capabilities: [], initialFiles: [] };
    const { rerender } = render(<AgentPreview {...props} harnessId="harness_first" />);
    expect(mutation.mutate.mock.calls[0][0]).toMatchObject({ harness_id: "harness_first" });

    rerender(<AgentPreview {...props} harnessId="harness_second" />);
    expect(mutation.mutate.mock.calls.at(-1)?.[0]).toMatchObject({ harness_id: "harness_second" });
  });

  it("shows included session features returned by the effective preview", () => {
    mockUsePreviewAgent.mockReturnValue(makeMutationStub({ data: sampleResponse }));
    render(<AgentPreview systemPrompt="hi" capabilities={[]} initialFiles={[]} />);
    expect(screen.getByText("Included Features")).toBeInTheDocument();
    expect(screen.getByText("Session filesystem")).toBeInTheDocument();
    expect(screen.getByText("Secrets")).toBeInTheDocument();
    expect(screen.getByText("Key-value storage")).toBeInTheDocument();
  });

  it("uses inherited initial files from the response and sends draft files and MCP servers", () => {
    const file = {
      path: "/inherited.txt",
      content: "Inherited content",
      encoding: "text" as const,
      is_readonly: true,
    };
    const mutation = makeMutationStub({ data: { ...sampleResponse, initial_files: [file] } });
    mockUsePreviewAgent.mockReturnValue(mutation);
    const mcpServers = { docs: { url: "https://example.com/mcp" } };
    render(
      <AgentPreview
        systemPrompt="hi"
        capabilities={[]}
        initialFiles={[]}
        mcpServers={mcpServers}
      />,
    );
    expect(mutation.mutate.mock.calls[0][0]).toMatchObject({ initial_files: [], mcpServers });
    expect(screen.getByText("Inherited content")).toBeInTheDocument();
  });

  it("ignores an older preview response after the harness changes", () => {
    const mutation = makeMutationStub({});
    mockUsePreviewAgent.mockReturnValue(mutation);
    const props = { systemPrompt: "hi", capabilities: [], initialFiles: [] };
    const { rerender } = render(<AgentPreview {...props} harnessId="first" />);
    const first = mutation.mutate.mock.calls[0][1];
    rerender(<AgentPreview {...props} harnessId="second" />);
    const second = mutation.mutate.mock.calls[1][1];
    act(() => {
      second?.onSuccess?.({ ...sampleResponse, system_prompt: "Current harness" });
      first?.onSuccess?.({ ...sampleResponse, system_prompt: "Stale harness" });
    });
    expect(screen.getByText("Current harness")).toBeInTheDocument();
    expect(screen.queryByText("Stale harness")).not.toBeInTheDocument();
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
