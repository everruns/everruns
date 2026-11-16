import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MonitoringCellsView, nodeRows } from "@/components/monitoring/cells";
import { getMonitoringCellDetail, getMonitoringCells } from "@/lib/api/monitoring";
import type { MonitoringCell } from "@/lib/api/types";

global.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

jest.mock("@/lib/api/monitoring", () => ({
  getMonitoringCells: jest.fn(),
  getMonitoringCellDetail: jest.fn(),
}));

const unavailable = {
  cell: "cell-a",
  available: false,
  reason: "No detail store is configured (MONITORING_DETAIL).",
  source: null,
  step_seconds: 10,
  from: "2026-11-15T19:00:00Z",
  to: "2026-11-15T20:00:00Z",
  nodes: [],
};

beforeEach(() => {
  jest.mocked(getMonitoringCellDetail).mockResolvedValue(unavailable);
});

function cell(name: string, extra: Partial<MonitoringCell> = {}): MonitoringCell {
  return {
    cell: name,
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
      <MonitoringCellsView />
    </QueryClientProvider>,
  );
}

describe("MonitoringCellsView", () => {
  it("lists cells with their load and flags silent cells and missing nodes", async () => {
    jest.mocked(getMonitoringCells).mockResolvedValue({
      generated_at: "2026-11-15T20:00:00Z",
      from: "2026-11-15T14:00:00Z",
      to: "2026-11-15T20:00:00Z",
      bucket_seconds: 60,
      cells: [cell("cell-a"), cell("cell-b", { silent: true, nodes_answering: 2, running: 20 })],
    });
    renderView();
    expect(await screen.findByRole("button", { name: "cell-a" })).toBeInTheDocument();
    expect(screen.getByText("silent")).toBeInTheDocument();
    expect(screen.getByText("2/3")).toBeInTheDocument();
    expect(screen.getAllByText("75%").length).toBeGreaterThan(0);
    expect(screen.getByText("2 cells")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "cell-b" }));
    expect(screen.getByRole("heading", { name: "cell-b" })).toBeInTheDocument();
    expect(getMonitoringCells).toHaveBeenCalledWith("6h");
  });

  it("explains an empty fleet", async () => {
    jest.mocked(getMonitoringCells).mockResolvedValue({
      generated_at: "2026-11-15T20:00:00Z",
      from: "2026-11-15T14:00:00Z",
      to: "2026-11-15T20:00:00Z",
      bucket_seconds: 60,
      cells: [],
    });
    renderView();
    expect(await screen.findByText("No cells have reported")).toBeInTheDocument();
  });

  it("says why a cell has no 10-second detail, and shows it by node when it does", async () => {
    jest.mocked(getMonitoringCells).mockResolvedValue({
      generated_at: "2026-11-15T20:00:00Z",
      from: "2026-11-15T14:00:00Z",
      to: "2026-11-15T20:00:00Z",
      bucket_seconds: 60,
      cells: [cell("cell-a")],
    });
    const view = renderView();
    expect(await screen.findByTestId("cell-detail-unavailable")).toHaveTextContent(
      "MONITORING_DETAIL",
    );
    view.unmount();
    jest.mocked(getMonitoringCellDetail).mockResolvedValue({
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
    expect(getMonitoringCellDetail).toHaveBeenCalledWith("cell-a");
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
