import { render, screen } from "@testing-library/react";
import { LlmHistoryViewer } from "@/components/llm/llm-history-viewer";
import type { LlmGenerationData, TokenUsage } from "@/lib/api/types";
import type { ReactNode } from "react";

jest.mock("@/components/ui/scroll-area", () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

describe("generation cost provenance", () => {
  it.each([
    [{ actual_cost_usd: 1, effective_cost_usd: 1 }, false],
    [{ actual_cost_usd: 1, estimated_cost_usd: 0.9, effective_cost_usd: 1.25 }, true],
    [{ estimated_cost_usd: 1.25, effective_cost_usd: 1.25 }, true],
  ] as const)("labels the effective total according to its cost basis", (cost, estimated) => {
    const usage: TokenUsage = { input_tokens: 100, output_tokens: 20, ...cost };
    const data: LlmGenerationData = {
      messages: [],
      tools: [],
      output: { text: "Ready", tool_calls: [] },
      metadata: { model: "gpt-6.2-sol", success: true, usage },
    };
    render(<LlmHistoryViewer data={data} />);
    const badge = screen.getByTitle(estimated ? "Price-table estimate" : "Provider-reported cost");
    expect(badge).toHaveTextContent(estimated ? "$1.25" : "$1.00");
    expect(badge.textContent?.includes("est.")).toBe(estimated);
  });
});
