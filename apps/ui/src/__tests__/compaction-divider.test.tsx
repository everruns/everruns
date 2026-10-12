import { fireEvent, render, screen } from "@testing-library/react";
import { CompactionDivider, type CompactionMarkerData } from "@/components/chat/compaction-divider";

function data(overrides: Partial<CompactionMarkerData>): CompactionMarkerData {
  return {
    strategy_used: "observation_masking",
    messages_before: 120,
    messages_after: 12,
    duration_ms: 40,
    model: "gpt-6-astra",
    ...overrides,
  } as CompactionMarkerData;
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

  it("shows tokens, not provider item counts, for a native compaction", () => {
    render(
      <CompactionDivider
        data={data({
          strategy_used: "native",
          messages_before: 179,
          messages_after: 188,
          tokens_before: 152_000,
          tokens_after: 9_400,
        })}
      />,
    );
    expect(screen.queryByText(/messages/)).not.toBeInTheDocument();
    expect(screen.getByText(/152.0K → 9.4K tokens · native/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.queryByText(/Saved:/)).not.toBeInTheDocument();
  });
});
