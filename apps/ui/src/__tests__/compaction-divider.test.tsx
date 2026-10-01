import { fireEvent, render, screen } from "@testing-library/react";
import { CompactionDivider } from "@/components/chat/compaction-divider";
import type { ContextCompactedData } from "@/lib/api/types";

function data(overrides: Partial<ContextCompactedData>): ContextCompactedData {
  return {
    strategy_used: "native",
    messages_before: 120,
    messages_after: 12,
    duration_ms: 40,
    model: "gpt-6-astra",
    ...overrides,
  } as ContextCompactedData;
}

describe("CompactionDivider", () => {
  it("shows the message counts of a local compaction", () => {
    render(<CompactionDivider data={data({})} />);
    expect(screen.getByText(/120 → 12 messages/)).toBeInTheDocument();
  });

  it("does not claim 0 → 0 messages when the counts are unknown", () => {
    render(
      <CompactionDivider
        data={data({
          strategy_used: "provider_managed",
          messages_before: 0,
          messages_after: 0,
        })}
      />,
    );
    expect(screen.getByText(/Context compacted · provider_managed/)).toBeInTheDocument();
    expect(screen.queryByText(/→/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.queryByText(/Saved:/)).not.toBeInTheDocument();
  });
});
