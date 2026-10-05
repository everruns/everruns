import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SandboxDetailDrawer } from "@/components/sandboxes/sandbox-detail-drawer";
import { SandboxFleetSummary } from "@/components/sandboxes/sandbox-fleet-summary";
import { SandboxFleetTable } from "@/components/sandboxes/sandbox-fleet-table";
import { SandboxTimelineChart } from "@/components/sandboxes/sandbox-timeline";
import { api } from "@/lib/api/client";
import {
  listSandboxes,
  type SandboxFleetDetail,
  type SandboxFleetItem,
  type SandboxFleetStats,
} from "@/lib/api/sandboxes";

jest.mock("@/lib/api/client", () => ({
  api: { get: jest.fn(), post: jest.fn() },
}));

const mockedGet = api.get as jest.Mock;
const mockedPost = api.post as jest.Mock;

function item(overrides: Partial<SandboxFleetItem> = {}): SandboxFleetItem {
  return {
    id: "sandbox_1",
    state: "running",
    desired_state: "ready",
    provider: "daytona",
    target_kind: "managed",
    role: "primary",
    session_id: "session_1",
    session_title: "Fix flaky CI",
    agent_id: "agent_1",
    agent_name: "Coder",
    template: { id: "sbxtpl_1", display_name: "Coding", revision: 4 },
    generation: 1,
    checkpoint_count: 12,
    external_id: "dtn-1",
    workspace_path: "/home/daytona/workspace",
    idle_after_seconds: 300,
    last_init_error: null,
    attention: [],
    created_at: new Date(Date.now() - 3 * 3_600_000).toISOString(),
    updated_at: new Date().toISOString(),
    last_activity_at: new Date().toISOString(),
    deleted_at: null,
    ...overrides,
  };
}

function withClient(node: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>;
}

afterEach(() => {
  mockedGet.mockReset();
  mockedPost.mockReset();
});

describe("Sandbox fleet API", () => {
  it("sends filters and paging as query parameters, leaving out the defaults", async () => {
    mockedGet.mockResolvedValue({
      data: { items: [], total: 0, limit: 50, offset: 0 },
    });
    await listSandboxes(
      { state: "all", provider: "modal", needsAttention: true, search: " ci " },
      { limit: 50, offset: 100 },
    );
    const url = mockedGet.mock.calls[0][0] as string;
    const params = new URL(url, "http://x").searchParams;
    expect(params.get("state")).toBeNull();
    expect(params.get("provider")).toBe("modal");
    expect(params.get("needs_attention")).toBe("true");
    expect(params.get("search")).toBe("ci");
    expect(params.get("offset")).toBe("100");
  });
});

describe("SandboxFleetSummary", () => {
  const stats: SandboxFleetStats = {
    window_days: 7,
    by_state: [
      { key: "running", count: 5 },
      { key: "paused", count: 9 },
      { key: "deleted", count: 198 },
    ],
    live_by_provider: [
      { key: "daytona", count: 11 },
      { key: "modal", count: 3 },
    ],
    created_in_window: 24,
    created_in_prior_window: 20,
    running_seconds_in_window: 96.4 * 3600,
    recoveries_in_window: 3,
    needs_attention: 2,
  };

  it("adds up live Sandboxes and names providers", () => {
    render(
      <SandboxFleetSummary
        stats={stats}
        state="live"
        needsAttention={false}
        onState={jest.fn()}
        onNeedsAttention={jest.fn()}
      />,
    );
    expect(screen.getByText("14")).toBeInTheDocument();
    expect(screen.getByText("5 running, 9 paused")).toBeInTheDocument();
    expect(screen.getByText("Daytona 11, Modal 3")).toBeInTheDocument();
    expect(screen.getByText("96.4 h")).toBeInTheDocument();
    expect(screen.getByText("up 20% on the week before")).toBeInTheDocument();
  });

  it("turns figures into filters", () => {
    const onState = jest.fn();
    const onNeedsAttention = jest.fn();
    render(
      <SandboxFleetSummary
        stats={stats}
        state="live"
        needsAttention={false}
        onState={onState}
        onNeedsAttention={onNeedsAttention}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Needs attention/ }));
    expect(onNeedsAttention).toHaveBeenCalledWith(true);
    fireEvent.click(screen.getByRole("button", { name: /Deleted 198/ }));
    expect(onState).toHaveBeenCalledWith("deleted");
  });
});

describe("SandboxFleetTable", () => {
  it("keeps a deleted Session's Sandbox readable and flags rebuilt ones", () => {
    const onSelect = jest.fn();
    render(
      <SandboxFleetTable
        items={[
          item({
            id: "sandbox_a",
            generation: 3,
            attention: ["lost"],
            state: "lost",
          }),
          item({
            id: "sandbox_b",
            state: "deleted",
            session_id: null,
            session_title: null,
            deleted_at: new Date().toISOString(),
          }),
        ]}
        selectedId={null}
        onSelect={onSelect}
      />,
    );
    expect(screen.getByText("Deleted session")).toBeInTheDocument();
    expect(screen.getByText("3")).toHaveClass("text-warning");
    expect(screen.getByLabelText("Lost, not rebuilt yet")).toBeInTheDocument();
    fireEvent.click(screen.getAllByText("Fix flaky CI")[0]);
    expect(onSelect).toHaveBeenCalledWith("sandbox_a");
  });
});

describe("SandboxTimelineChart", () => {
  it("draws a lane per Sandbox and marks rebuilds", () => {
    const from = new Date("2026-10-05T00:00:00Z");
    const at = (hours: number) => new Date(from.getTime() + hours * 3_600_000).toISOString();
    render(
      <SandboxTimelineChart
        timeline={{
          from: at(0),
          to: at(24),
          lanes: [
            {
              sandbox: item(),
              running_seconds: 7200,
              spans: [
                {
                  state: "running",
                  generation: 1,
                  start: at(1),
                  end: at(2),
                  current: false,
                },
                {
                  state: "lost",
                  generation: 1,
                  start: at(2),
                  end: at(3),
                  current: false,
                },
                {
                  state: "running",
                  generation: 2,
                  start: at(3),
                  end: at(4),
                  current: true,
                },
              ],
            },
          ],
          total_lanes: 3,
          concurrency: [
            { at: at(0), running: 0 },
            { at: at(1), running: 1 },
          ],
          peak_running: 1,
          peak_at: at(1),
        }}
        onSelect={jest.fn()}
      />,
    );
    expect(screen.getByLabelText("Fix flaky CI lifecycle")).toBeInTheDocument();
    expect(screen.getByTitle("Rebuilt as generation 2")).toBeInTheDocument();
    expect(screen.getByText("Showing the 1 longest-running of 3")).toBeInTheDocument();
  });
});

describe("SandboxDetailDrawer", () => {
  function detail(overrides: Partial<SandboxFleetDetail> = {}): SandboxFleetDetail {
    return { ...item(), incarnations: [], history: [], ...overrides };
  }

  it("asks before deleting and sends the action to the Session", async () => {
    mockedGet.mockResolvedValue({ data: detail() });
    mockedPost.mockResolvedValue({ data: {} });
    render(withClient(<SandboxDetailDrawer sandboxId="sandbox_1" onClose={jest.fn()} />));

    fireEvent.click(await screen.findByRole("button", { name: /Delete/ }));
    expect(mockedPost).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Delete Sandbox and workspace" }));
    await waitFor(() =>
      expect(mockedPost).toHaveBeenCalledWith("/v1/sessions/session_1/sandbox", {
        action: "delete",
      }),
    );
  });

  it("offers no actions once the Session is gone", async () => {
    mockedGet.mockResolvedValue({
      data: detail({
        state: "deleted",
        session_id: null,
        deleted_at: new Date().toISOString(),
      }),
    });
    render(withClient(<SandboxDetailDrawer sandboxId="sandbox_1" onClose={jest.fn()} />));
    expect(await screen.findByText(/Its Session was deleted/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Pause|Resume|Delete/ })).toBeNull();
  });

  it("explains why a Sandbox needs attention", async () => {
    mockedGet.mockResolvedValue({
      data: detail({
        attention: ["init_failed"],
        last_init_error: "pip exited 1",
      }),
    });
    render(withClient(<SandboxDetailDrawer sandboxId="sandbox_1" onClose={jest.fn()} />));
    expect(await screen.findByText("Init commands failed")).toBeInTheDocument();
    expect(screen.getByText("pip exited 1")).toBeInTheDocument();
  });
});
