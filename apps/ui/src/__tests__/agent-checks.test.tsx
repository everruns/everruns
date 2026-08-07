import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { AgentChecks, applyByteSpanReplacement } from "@/components/agents/agent-checks";
import type { AgentAnalysisResponse, AgentPreviewResponse } from "@/lib/api/types";

const mockUsePreviewAgent = jest.fn();
const mockUseAnalyzeAgent = jest.fn();

jest.mock("@/hooks/use-agents", () => ({
  usePreviewAgent: () => mockUsePreviewAgent(),
  useAnalyzeAgent: () => mockUseAnalyzeAgent(),
}));

function previewResponse(message?: string): AgentPreviewResponse {
  return {
    system_prompt: "resolved prompt",
    tools: [],
    findings: message
      ? [
          {
            rule_id: "prompt.template_variables",
            severity: "warning",
            category: "structure",
            message,
            source: "builtin",
          },
        ]
      : [],
  };
}

function mutationStub({
  isPending = false,
  error = null,
}: {
  isPending?: boolean;
  error?: Error | null;
} = {}) {
  return {
    mutate: jest.fn(),
    reset: jest.fn(),
    isPending,
    error,
  };
}

describe("AgentChecks", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAnalyzeAgent.mockReturnValue(mutationStub());
  });

  it("shows a loading state while built-in checks refresh", () => {
    mockUsePreviewAgent.mockReturnValue(mutationStub({ isPending: true }));

    render(<AgentChecks systemPrompt="hi" capabilities={[]} />);

    expect(screen.getByLabelText("Refreshing checks")).toBeInTheDocument();
  });

  it("renders built-in findings and keeps the request in sync with edited config", async () => {
    const preview = mutationStub();
    preview.mutate.mockImplementation(
      (
        request: { system_prompt: string },
        callbacks: { onSuccess: (data: AgentPreviewResponse) => void },
      ) => callbacks.onSuccess(previewResponse(`Finding for ${request.system_prompt}`)),
    );
    mockUsePreviewAgent.mockReturnValue(preview);

    const { rerender } = render(<AgentChecks systemPrompt="first" capabilities={[]} />);
    await waitFor(() => expect(screen.getByText("Finding for first")).toBeInTheDocument());

    rerender(<AgentChecks systemPrompt="second" capabilities={[]} />);
    rerender(<AgentChecks systemPrompt="third" capabilities={[]} />);
    await waitFor(() => expect(screen.getByText("Finding for third")).toBeInTheDocument());

    expect(preview.mutate).toHaveBeenCalledTimes(2);
    expect(preview.mutate).toHaveBeenLastCalledWith(
      { system_prompt: "third", capabilities: [], tools: [] },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("ignores a late built-in response for an older edit", async () => {
    const callbacks: Array<{ onSuccess: (data: AgentPreviewResponse) => void }> = [];
    const preview = mutationStub();
    preview.mutate.mockImplementation((_request: unknown, nextCallbacks: (typeof callbacks)[0]) => {
      callbacks.push(nextCallbacks);
    });
    mockUsePreviewAgent.mockReturnValue(preview);

    const { rerender } = render(<AgentChecks systemPrompt="first" capabilities={[]} />);
    await waitFor(() => expect(callbacks).toHaveLength(1));
    rerender(<AgentChecks systemPrompt="second" capabilities={[]} />);
    await waitFor(() => expect(callbacks).toHaveLength(2));

    act(() => callbacks[1].onSuccess(previewResponse("Current finding")));
    act(() => callbacks[0].onSuccess(previewResponse("Stale finding")));

    expect(screen.getByText("Current finding")).toBeInTheDocument();
    expect(screen.queryByText("Stale finding")).not.toBeInTheDocument();
  });

  it("runs deeper analysis and replaces the built-in findings", async () => {
    const preview = mutationStub();
    preview.mutate.mockImplementation(
      (_request: unknown, callbacks: { onSuccess: (data: AgentPreviewResponse) => void }) =>
        callbacks.onSuccess(previewResponse("Built-in finding")),
    );
    const analyze = mutationStub();
    analyze.mutate.mockImplementation(
      (_request: unknown, callbacks: { onSuccess: (data: AgentAnalysisResponse) => void }) =>
        callbacks.onSuccess({
          findings: [
            {
              rule_id: "prompt.structure",
              severity: "suggestion",
              category: "structure",
              message: "AI finding",
              source: "llm",
            },
          ],
        }),
    );
    mockUsePreviewAgent.mockReturnValue(preview);
    mockUseAnalyzeAgent.mockReturnValue(analyze);

    render(<AgentChecks systemPrompt="hi" capabilities={[]} />);
    await waitFor(() => expect(screen.getByText("Built-in finding")).toBeInTheDocument());

    fireEvent.click(screen.getByRole("button", { name: "Analyze" }));

    expect(await screen.findByText("AI finding")).toBeInTheDocument();
    expect(screen.queryByText("Built-in finding")).not.toBeInTheDocument();
    expect(analyze.mutate).toHaveBeenCalledWith(
      { system_prompt: "hi", capabilities: [], tools: [] },
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("preserves built-in findings when deeper analysis fails", async () => {
    const preview = mutationStub();
    preview.mutate.mockImplementation(
      (_request: unknown, callbacks: { onSuccess: (data: AgentPreviewResponse) => void }) =>
        callbacks.onSuccess(previewResponse("Built-in finding")),
    );
    mockUsePreviewAgent.mockReturnValue(preview);
    mockUseAnalyzeAgent.mockReturnValue(mutationStub({ error: new Error("utility unavailable") }));

    render(<AgentChecks systemPrompt="hi" capabilities={[]} />);

    expect(await screen.findByText("Built-in finding")).toBeInTheDocument();
    expect(screen.getByText(/Analysis failed: utility unavailable/)).toBeInTheDocument();
  });

  it("applies a proposed system-prompt fix", async () => {
    const onApplyFix = jest.fn();
    const preview = mutationStub();
    preview.mutate.mockImplementation(
      (_request: unknown, callbacks: { onSuccess: (data: AgentPreviewResponse) => void }) =>
        callbacks.onSuccess({
          ...previewResponse(),
          findings: [
            {
              rule_id: "prompt.duplicate",
              severity: "suggestion",
              category: "structure",
              message: "Replace duplicate guidance.",
              source: "builtin",
              location: { field: "system_prompt", start: 7, end: 10 },
              fix: "good",
            },
          ],
        }),
    );
    mockUsePreviewAgent.mockReturnValue(preview);

    render(<AgentChecks systemPrompt="héllo bad end" capabilities={[]} onApplyFix={onApplyFix} />);
    fireEvent.click(await screen.findByRole("button", { name: "Apply fix" }));

    expect(onApplyFix).toHaveBeenCalledWith(7, 10, "good");
  });

  it("shows the built-in check error state", () => {
    mockUsePreviewAgent.mockReturnValue(mutationStub({ error: new Error("preview unavailable") }));

    render(<AgentChecks systemPrompt="hi" capabilities={[]} />);

    expect(screen.getByText("Checks Error")).toBeInTheDocument();
    expect(screen.getByText(/preview unavailable/)).toBeInTheDocument();
  });
});

describe("applyByteSpanReplacement", () => {
  it("replaces an ascii byte span", () => {
    expect(applyByteSpanReplacement("Use `search_web` now.", 4, 16, "`web_fetch`")).toBe(
      "Use `web_fetch` now.",
    );
  });

  it("handles multibyte text before the span", () => {
    expect(applyByteSpanReplacement("héllo bad end", 7, 10, "good")).toBe("héllo good end");
  });
});
