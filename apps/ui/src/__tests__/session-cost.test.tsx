import { fireEvent, render, screen } from "@testing-library/react";
import { LocaleProvider } from "@/providers/locale-provider";
import type { CostHour, SessionCost } from "@/lib/api/types";
import {
  CostCategories,
  cacheSavings,
  promptTokens,
} from "@/components/session/cost/cost-categories";
import { CostChart, dayBuckets, niceMax } from "@/components/session/cost/cost-chart";
import { SourcesCard } from "@/components/session/cost/cost-activity";

function hour(at: string, uncached: number, calls = 1): CostHour {
  return {
    hour: at,
    uncached_input_usd: uncached,
    cache_read_usd: 0,
    cache_write_usd: 0,
    output_usd: 0,
    unattributed_usd: 0,
    model_calls: calls,
  };
}

const cost: SessionCost = {
  session_id: "session_1",
  total_usd: 10,
  model_calls: 4,
  unpriced_calls: 0,
  categories: [
    { key: "uncached_input", tokens: 90_000_000, usd: 9 },
    { key: "cache_read", tokens: 10_000_000, usd: 0.02 },
    { key: "cache_write", tokens: 0, usd: 0 },
    { key: "output", tokens: 1_000_000, usd: 0.98 },
  ],
  unattributed_usd: 0,
  models: [{ model: "muse", provider: "openrouter", calls: 4, usd: 10 }],
  hours: [hour("2026-10-20T12:00:00Z", 4, 2), hour("2026-10-22T12:00:00Z", 6, 2)],
  turns: [
    {
      turn: 1,
      turn_id: "turn_1",
      started_at: "2026-10-20T12:00:00Z",
      status: "completed",
      usd: 4,
      prompt_tokens: 50_000_000,
      source_kind: "schedule",
    },
    {
      turn: 2,
      turn_id: "turn_2",
      started_at: "2026-10-22T12:00:00Z",
      status: "failed",
      usd: 6,
      prompt_tokens: 50_000_000,
      source_kind: "user",
    },
  ],
  turns_truncated: false,
  sources: [
    {
      kind: "schedule",
      source_id: "sched_1",
      name: "Claim poll",
      cron: "*/5 * * * *",
      active: false,
      turns: 1,
      usd: 4,
    },
    { kind: "user", source_id: null, name: null, cron: null, active: null, turns: 1, usd: 6 },
  ],
  tools: { calls: 0, failed: 0, duration_ms: 0, items: [] },
  sandboxes: [],
};

function wrap(node: React.ReactNode) {
  return render(<LocaleProvider>{node}</LocaleProvider>);
}

describe("session cost", () => {
  it("rolls hours into days and fills the days without calls", () => {
    const days = dayBuckets(cost.hours);
    expect(days).toHaveLength(3);
    expect(days.map((d) => d.calls)).toEqual([2, 0, 2]);
    expect(days[2].usd.uncached_input).toBe(6);
  });

  it("rounds the axis to a readable maximum", () => {
    expect(niceMax(0)).toBe(1);
    expect(niceMax(3.2)).toBe(5);
    expect(niceMax(18)).toBe(20);
    expect(niceMax(110)).toBe(200);
  });

  it("prices cached reads against the uncached rate", () => {
    expect(promptTokens(cost)).toBe(100_000_000);
    // 10M tokens at $0.10/M uncached is $1, minus the $0.02 they cost.
    expect(cacheSavings(cost)).toBeCloseTo(0.98);
  });

  it("shows the total and every category", () => {
    wrap(<CostCategories cost={cost} modelLabel="muse" />);
    expect(screen.getByText("$10.00")).toBeInTheDocument();
    expect(screen.getByText("Prompt, uncached")).toBeInTheDocument();
    expect(screen.getByText("Prompt, read from cache")).toBeInTheDocument();
  });

  it("switches the chart between days and turns", () => {
    wrap(<CostChart cost={cost} />);
    expect(screen.getByRole("heading", { name: "Spend per day" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Turn" }));
    expect(screen.getByRole("button", { name: "Turn" })).toHaveAttribute("aria-pressed", "true");
  });

  it("names a schedule that no longer fires", () => {
    wrap(<SourcesCard cost={cost} />);
    expect(screen.getByText("· Claim poll")).toBeInTheDocument();
    expect(screen.getByText("*/5 * * * * · no longer runs")).toBeInTheDocument();
  });
});
