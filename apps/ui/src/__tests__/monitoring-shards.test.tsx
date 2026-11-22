import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MonitoringShardsView, nodeRows } from "@/components/monitoring/shards";
import { getMonitoringShardDetail, getMonitoringShards } from "@/lib/api/monitoring";
import type { MonitoringShard } from "@/lib/api/types";

global.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

jest.mock("@/lib/api/monitoring", () => ({
  getMonitoringShards: jest.fn(),
  getMonitoringShardDetail: jest.fn(),
}));

const unavailable = {
  shard: "shard-a",
  available: false,
  reason: "No detail store is configured (MONITORING_DETAIL).",
  source: null,
  step_seconds: 10,
  from: "2026-11-15T19:00:00Z",
  to: "2026-11-15T20:00:00Z",
  nodes: [],
};

beforeEach(() => {
  jest.mocked(getMonitoringShardDetail).mockResolvedValue(unavailable);
});

function shard(name: string, extra: Partial<MonitoringShard> = {}): MonitoringShard {
  return {
    shard: name,
    data_plane: "dp-1",
    reported_at: "2026-11-15T19:59:00Z",
    silent: false,
    leader: "node-1",
    nodes: 3,
    nodes_answering: 3,
    sessions: 1200,
    running: 80,
    queued: 0,
    resumed: 1,
    memory_bytes: 3 * 1024 ** 3,
    memory_limit_bytes: 4 * 1024 ** 3,
    cpu_cores: 1,
    cpu_limit_cores: 2,
    admit_p99_ms: 12,
    ship_lag_p99_ms: 40,
    term: 4,
    buckets: [],
    ...extra,
  };
}

function renderView() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MonitoringShardsView />
    </QueryClientProvider>,
  );
}

describe("MonitoringShardsView", () => {
  it("lists shards with their load and flags silent shards and missing nodes", async () => {
    jest.mocked(getMonitoringShards).mockResolvedValue({
      generated_at: "2026-11-15T20:00:00Z",
      from: "2026-11-15T14:00:00Z",
      to: "2026-11-15T20:00:00Z",
      bucket_seconds: 60,
      shards: [
        shard("shard-a"),
        shard("shard-b", { silent: true, nodes_answering: 2, running: 20 }),
      ],
    });
    renderView();
    expect(await screen.findByRole("button", { name: "shard-a" })).toBeInTheDocument();
    expect(screen.getByText("silent")).toBeInTheDocument();
    expect(screen.getByText("2/3")).toBeInTheDocument();
    expect(screen.getAllByText("75%").length).toBeGreaterThan(0);
    expect(screen.getByText("2 shards")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "shard-b" }));
    expect(screen.getByRole("heading", { name: "shard-b" })).toBeInTheDocument();
    expect(getMonitoringShards).toHaveBeenCalledWith("6h");
  });

  it("explains an empty fleet", async () => {
    jest.mocked(getMonitoringShards).mockResolvedValue({
      generated_at: "2026-11-15T20:00:00Z",
      from: "2026-11-15T14:00:00Z",
      to: "2026-11-15T20:00:00Z",
      bucket_seconds: 60,
      shards: [],
    });
    renderView();
    expect(await screen.findByText("No shards have reported")).toBeInTheDocument();
  });

  it("says why a shard has no 10-second detail, and shows it by node when it does", async () => {
    jest.mocked(getMonitoringShards).mockResolvedValue({
      generated_at: "2026-11-15T20:00:00Z",
      from: "2026-11-15T14:00:00Z",
      to: "2026-11-15T20:00:00Z",
      bucket_seconds: 60,
      shards: [shard("shard-a")],
    });
    const view = renderView();
    expect(await screen.findByTestId("shard-detail-unavailable")).toHaveTextContent(
      "MONITORING_DETAIL",
    );
    view.unmount();
    jest.mocked(getMonitoringShardDetail).mockResolvedValue({
      ...unavailable,
      available: true,
      reason: null,
      source: "http://victoriametrics:8428",
      nodes: [
        {
          node: "1",
          points: [
            { t: "2026-11-15T19:59:50Z", running: 3, rss_bytes: 1024, queued: 0, admit_p99_ms: 9 },
            { t: "2026-11-15T20:00:00Z", running: 4, rss_bytes: 2048, queued: 0, admit_p99_ms: 9 },
          ],
        },
      ],
    });
    renderView();
    expect(await screen.findByText(/from http:\/\/victoriametrics:8428/)).toBeInTheDocument();
    expect(getMonitoringShardDetail).toHaveBeenCalledWith("shard-a");
  });
});

describe("nodeRows", () => {
  it("puts each node in its own column, one row per time", () => {
    const rows = nodeRows(
      {
        ...unavailable,
        available: true,
        nodes: [
          { node: "1", points: [{ t: "2026-11-15T20:00:00Z", running: 3 }] },
          { node: "2", points: [{ t: "2026-11-15T20:00:00Z", running: 5 }] },
        ],
      },
      (p) => p.running,
    );
    expect(rows).toEqual([{ t: Date.parse("2026-11-15T20:00:00Z"), n1: 3, n2: 5 }]);
  });
});
