import { render, screen, waitFor } from "@testing-library/react";
import { HarnessPreview } from "@/components/harnesses/harness-preview";
import type { AgentPreviewResponse } from "@/lib/api/types";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => (
    <pre data-testid="streamdown-mock">{children}</pre>
  ),
}));

jest.mock("@streamdown/code", () => ({
  code: {},
}));

const mockUsePreviewHarness = jest.fn();

jest.mock("@/hooks/use-harnesses", () => ({
  usePreviewHarness: () => mockUsePreviewHarness(),
}));

function makeMutationStub(opts: {
  data?: AgentPreviewResponse | null;
  isPending?: boolean;
  error?: Error | null;
}) {
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
  system_prompt: "## Harness prompt\n\nDo good things.",
  tools: [
    {
      type: "builtin",
      name: "shell",
      description: "Run a shell command",
      parameters: { type: "object", properties: {} },
    },
  ],
};

describe("HarnessPreview", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("shows skeletons while the preview request is pending", () => {
    mockUsePreviewHarness.mockReturnValue(makeMutationStub({ isPending: true }));

    render(
      <HarnessPreview
        systemPrompt="hi"
        capabilities={[]}
        initialFiles={[]}
      />,
    );

    expect(screen.queryByText("Full System Prompt")).not.toBeInTheDocument();
  });

  it("renders system prompt, tools, and initial files preview on success", async () => {
    mockUsePreviewHarness.mockReturnValue(makeMutationStub({ data: sampleResponse }));

    render(
      <HarnessPreview
        systemPrompt="base prompt"
        capabilities={[]}
        initialFiles={[
          {
            path: "/welcome.md",
            content: "welcome to the harness",
            encoding: "text",
            is_readonly: true,
          },
        ]}
      />,
    );

    await waitFor(() => expect(screen.getByText("Full System Prompt")).toBeInTheDocument());
    expect(screen.getByText(/Do good things\./)).toBeInTheDocument();
    expect(screen.getByText("Available Tools")).toBeInTheDocument();
    expect(screen.getByText("shell")).toBeInTheDocument();
    expect(screen.getByText("Initial Files")).toBeInTheDocument();
    expect(screen.getByText("welcome to the harness")).toBeInTheDocument();
    expect(screen.getByText("read-only")).toBeInTheDocument();
  });

  it("shows the error card when the preview mutation fails", () => {
    mockUsePreviewHarness.mockReturnValue(
      makeMutationStub({ error: new Error("boom") }),
    );

    render(
      <HarnessPreview
        systemPrompt="hi"
        capabilities={[]}
        initialFiles={[]}
      />,
    );

    expect(screen.getByText("Preview Error")).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  // Regression for "TypeError: t is not iterable" in the harness preview tab when an
  // older harness record arrives without the `initial_files` field.
  it.each([undefined, null])(
    "does not crash when harness.initial_files is %p",
    (initialFiles) => {
      mockUsePreviewHarness.mockReturnValue(makeMutationStub({ data: sampleResponse }));

      render(
        <HarnessPreview
          systemPrompt="hi"
          capabilities={[]}
          initialFiles={initialFiles as never}
        />,
      );

      expect(screen.getByText("Initial Files")).toBeInTheDocument();
      expect(screen.getByText("No initial files configured.")).toBeInTheDocument();
    },
  );

  it("passes parent_harness_id through to the preview mutation", () => {
    const stub = makeMutationStub({ data: sampleResponse });
    mockUsePreviewHarness.mockReturnValue(stub);

    render(
      <HarnessPreview
        systemPrompt="hi"
        capabilities={[]}
        initialFiles={[]}
        parentHarnessId="harness-parent-1"
      />,
    );

    expect(stub.mutate).toHaveBeenCalledWith(
      expect.objectContaining({ parent_harness_id: "harness-parent-1" }),
      expect.any(Object),
    );
  });
});
