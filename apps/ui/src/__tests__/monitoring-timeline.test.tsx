import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MonitoringTimelineView } from "@/components/monitoring/timeline";
import { selectionTotals, timelineRows, change } from "@/components/monitoring/timeline-model";
import { getMonitoringTimeline, getMonitoringTimelineWindow } from "@/lib/api/monitoring";
import type { MonitoringTimeline, TimelineBucket } from "@/lib/api/types";

// jsdom has no ResizeObserver; charts render empty at zero size, which is fine here.
global.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

jest.mock("@/lib/api/monitoring", () => ({
  getMonitoringTimeline: jest.fn(),
  getMonitoringTimelineWindow: jest.fn(),
}));
jest.mock("next/navigation", () => ({ usePathname: () => "/monitoring" }));

function bucket(start: string, extra: Partial<TimelineBucket> = {}): TimelineBucket {
  return {
    start,
    sampled: true,
    backfilled: false,
    active_avg: 10,
    active_peak: 12,
    model_avg: 5,
    tools_avg: 3,
    waiting_avg: 1,
    runnable_avg: 1,
    setup_avg: 0,
    runnable_peak: 2,
    oldest_runnable_ms: 1500,
    leases_peak: 9,
    lease_takeovers: 0,
    turns_started: 40,
    turns_completed: 36,
    turns_failed: 2,
    turns_cancelled: 1,
    model_calls: 90,
    tool_calls: 120,
    tool_failures: 3,
    input_tokens: 1000,
    output_tokens: 200,
    cost_usd: 0.25,
    model_latency_p95_ms: 2100,
    organizations_peak: null,
    workers: null,
    worker_capacity: null,
    worker_load_avg: null,
    worker_load_peak: null,
    worker_loads: [],
    ...extra,
  };
}

function timeline(): MonitoringTimeline {
  return {
    generated_at: "2026-11-01T06:00:00Z",
    scope: "organization",
    from: "2026-11-01T05:00:00Z",
    to: "2026-11-01T05:03:00Z",
    bucket_seconds: 60,
    buckets: [
      bucket("2026-11-01T05:00:00Z", {
        sampled: false,
        backfilled: true,
        active_avg: 4,
        model_avg: 0,
        tools_avg: 0,
        waiting_avg: 0,
        runnable_avg: 0,
        oldest_runnable_ms: null,
      }),
      bucket("2026-11-01T05:01:00Z"),
      bucket("2026-11-01T05:02:00Z", { active_peak: 30, oldest_runnable_ms: 9800 }),
    ],
    summary: {
      peak_active: 30,
      peak_at: "2026-11-01T05:02:00Z",
      turns_started: 120,
      turns_finished: 117,
      turns_failed: 6,
      failure_rate: 6 / 117,
      tool_calls: 360,
      longest_wait_ms: 9800,
      longest_wait_at: "2026-11-01T05:02:00Z",
      cost_usd: 0.75,
      seconds_above_90: null,
    },
    previous: {
      peak_active: 20,
      peak_at: null,
      turns_started: 100,
      turns_finished: 90,
      turns_failed: 1,
      failure_rate: 1 / 90,
      tool_calls: 300,
      longest_wait_ms: null,
      longest_wait_at: null,
      cost_usd: 1,
      seconds_above_90: null,
    },
    markers: [{ at: "2026-11-01T05:01:30Z", kind: "deploy", label: "0.50.1" }],
  };
}

describe("timeline model", () => {
  it("draws rebuilt history as one area without a phase split", () => {
    const rows = timelineRows(timeline().buckets);
    expect(rows[0]).toMatchObject({ rebuilt: 4, model: 0, wait: null });
    expect(rows[1]).toMatchObject({ rebuilt: 0, model: 5, wait: 1.5, errors: 5 });
  });

  it("totals a dragged range from loaded buckets", () => {
    const t = selectionTotals(timeline(), { start: 1, end: 2 });
    expect(t).toMatchObject({
      peak: 30,
      turnsStarted: 80,
      turnsFailed: 4,
      longestWaitMs: 9800,
      from: "2026-11-01T05:01:00Z",
      to: "2026-11-01T05:03:00.000Z",
    });
    expect(t.phases.find((p) => p.key === "model")?.share).toBeCloseTo(0.5);
  });

  it("does not compare against an empty previous period", () => {
    expect(change(5, 0)).toBeNull();
    expect(change(15, 10)).toBeCloseTo(0.5);
  });
});

describe("MonitoringTimelineView", () => {
  beforeEach(() => {
    jest.mocked(getMonitoringTimeline).mockResolvedValue(timeline());
    jest.mocked(getMonitoringTimelineWindow).mockResolvedValue({
      from: "2026-11-01T05:00:00Z",
      to: "2026-11-01T05:03:00Z",
      busiest: [{ id: "agent_1", name: "Support bot", sessions: 7, turns: 30 }],
      failures: [{ code: "provider_rate_limited", turns: 5 }],
    });
  });

  function renderView() {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    return render(
      <QueryClientProvider client={client}>
        <MonitoringTimelineView scope="organization" />
      </QueryClientProvider>,
    );
  }

  it("shows summary tiles, the range breakdown, and switches ranges", async () => {
    renderView();
    const kpis = await screen.findByTestId("timeline-kpis");
    expect(kpis).toHaveTextContent("Peak parallel sessions30");
    expect(kpis).toHaveTextContent("+30% vs previous period");
    expect(kpis).toHaveTextContent("5.1%");
    expect(kpis).toHaveTextContent("9.8s");
    expect(await screen.findByText("Support bot")).toBeInTheDocument();
    expect(screen.getByText("provider rate limited")).toBeInTheDocument();
    expect(screen.getByText(/rebuilt from turn records/)).toBeInTheDocument();
    expect(getMonitoringTimeline).toHaveBeenCalledWith("organization", { range: "24h" });
    fireEvent.click(screen.getByRole("button", { name: "7d" }));
    await waitFor(() =>
      expect(getMonitoringTimeline).toHaveBeenCalledWith("organization", { range: "7d" }),
    );
  });
});
