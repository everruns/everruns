import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MonitoringCellsView } from "@/components/monitoring/cells";
import { getMonitoringCells } from "@/lib/api/monitoring";
import type { MonitoringCell } from "@/lib/api/types";

global.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

jest.mock("@/lib/api/monitoring", () => ({ getMonitoringCells: jest.fn() }));

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
});
