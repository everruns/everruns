import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MonitoringPage } from "@/components/monitoring/monitoring-page";
import { ExecutionFlow } from "@/components/monitoring/execution-flow";
import { WorkerGrid, groupWorkers } from "@/components/monitoring/worker-grid";
import { ActorDetail } from "@/components/monitoring/actors";
import { appendHistory, historyPoint, replaySnapshot } from "@/components/monitoring/model";
import { getMonitoring } from "@/lib/api/monitoring";
import { ApiError } from "@/lib/api/client";
import type { MonitoringSnapshot, MonitoringActorDetail } from "@/lib/api/types";

let mockEnabled = true;
let mockPlatform = true;
let mockOrg = "org_a";
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: mockOrg } }),
}));
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlagsState: () => ({ isLoading: false, flags: { monitoring: mockEnabled } }),
}));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({
    isLoading: false,
    can: (id: string) => id !== "durable.view" || mockPlatform,
  }),
}));
jest.mock("@/lib/api/monitoring", () => ({
  getMonitoring: jest.fn(),
  getMonitoringActor: jest.fn(),
  getMonitoringActors: jest.fn(),
  getMonitoringTimeline: jest.fn(),
  getMonitoringTimelineWindow: jest.fn(),
}));
jest.mock("next/navigation", () => ({
  usePathname: () => "/durable",
  useSearchParams: () => new URLSearchParams(),
}));

function snapshot(at = "2026-10-14T12:00:00Z", actors = 50000): MonitoringSnapshot {
  return {
    generated_at: at,
    scope: "fleet",
    window_seconds: 60,
    actors,
    organizations: 126,
    actor_leases: 20000,
    expired_leases: 7,
    oldest_runnable_ms: 8200,
    phases: [
      { phase: "runnable", count: 1000, oldest_ms: 8200 },
      { phase: "model", count: actors - 2000, oldest_ms: 12000 },
      { phase: "tools", count: 700, oldest_ms: 4000 },
      { phase: "waiting", count: 200, oldest_ms: 60000 },
      { phase: "setup", count: 100, oldest_ms: 10 },
    ],
    cohorts: [],
    ownership: [],
    workers: [],
    events: {
      "turn.started": 120,
      "turn.completed": 90,
      "reason.started": 600,
      "tool.started": 1200,
    },
    lease_events: { reacquired: 12 },
    cohorts_truncated: false,
    workers_truncated: false,
  };
}
function app(scope: "organization" | "fleet" = "fleet") {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return {
    client,
    element: (
      <QueryClientProvider client={client}>
        <MonitoringPage scope={scope} />
      </QueryClientProvider>
    ),
  };
}
beforeEach(() => {
  mockEnabled = true;
  mockPlatform = true;
  mockOrg = "org_a";
  jest.clearAllMocks();
  (getMonitoring as jest.Mock).mockResolvedValue(snapshot());
});

test("overview population does not create one rendered row per execution", () => {
  const { container } = render(
    <ExecutionFlow data={snapshot()} selected={null} onSelect={jest.fn()} />,
  );
  expect(container.querySelectorAll('g[role="button"]')).toHaveLength(5);
  expect(container.querySelectorAll("*").length).toBeLessThan(150);
  expect(screen.getByRole("button", { name: "Model: 48000 sessions" })).toBeInTheDocument();
});

test("worker grouping keeps ownership distinct from capacity and bounds graphical nodes", () => {
  const data = snapshot();
  data.workers = Array.from({ length: 1000 }, (_, i) => ({
    id: `worker-${String(i).padStart(4, "0")}`,
    worker_group: "pool",
    actor_leases: 100,
    phases: { model: 80, tools: 20 },
    status: "active",
    capacity: 50,
    current_load: 10,
    accepting_work: true,
    last_heartbeat_at: data.generated_at,
  }));
  const groups = groupWorkers(data.workers, 32);
  expect(groups).toHaveLength(32);
  expect(groups[0].leases).toBe(3200);
  expect(groups[0].capacity).toBe(1600);
  const { container } = render(
    <WorkerGrid data={data} size={32} selected={null} onSelect={jest.fn()} />,
  );
  expect(screen.getAllByRole("button")).toHaveLength(32);
  expect(container.querySelectorAll("svg path").length).toBeLessThan(200);
});

test("history is compact, bounded, and does not attach current workers to old observations", () => {
  let history: ReturnType<typeof historyPoint>[] = [];
  for (let i = 0; i < 1000; i++)
    history = appendHistory(history, historyPoint(snapshot(new Date(i * 1000).toISOString(), i)));
  expect(history).toHaveLength(900);
  expect(appendHistory(history, history.at(-1)!)).toBe(history);
  const live = snapshot();
  const replay = replaySnapshot(live, history[0]);
  expect(replay.actors).toBe(history[0].actors);
  expect(replay.workers).toEqual([]);
  expect(replay.ownership).toEqual([]);
  expect(replay.lease_events).toEqual(history[0].leaseEvents);
  expect(replay.organizations).toBe(history[0].organizations);
  expect(replay.phases[0].oldest_ms).toBe(history[0].phaseOldest[replay.phases[0].phase]);
});

test("organization adoption and platform authorization prevent fetching gated monitoring", () => {
  mockEnabled = false;
  const org = app("organization");
  const rendered = render(org.element);
  expect(screen.getByText("Monitoring is not enabled")).toBeInTheDocument();
  expect(getMonitoring).not.toHaveBeenCalled();
  rendered.unmount();
  mockPlatform = false;
  render(app().element);
  expect(screen.getByText("Platform access required")).toBeInTheDocument();
  expect(getMonitoring).not.toHaveBeenCalled();
});

test("pause freezes the viewed population while live collection continues", async () => {
  const { client, element } = app();
  render(element);
  await screen.findByText("50,000 active sessions");
  fireEvent.click(screen.getByRole("button", { name: "Pause" }));
  await act(async () => {
    client.setQueryData(["monitoring", "fleet", "org_a"], snapshot("2026-10-14T12:00:01Z", 12345));
  });
  expect(screen.getByText("50,000 active sessions")).toBeInTheDocument();
  expect(screen.queryByText("12,345 active sessions")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Live" }));
  expect(screen.getByText("12,345 active sessions")).toBeInTheDocument();
});

test("replay disables live ownership tabs and an org switch discards the old observation", async () => {
  const { client, element } = app();
  const rendered = render(element);
  await screen.findByText("50,000 active sessions");
  await act(async () => {
    client.setQueryData(["monitoring", "fleet", "org_a"], snapshot("2026-10-14T12:00:01Z", 12345));
  });
  await waitFor(() => expect(screen.getByRole("button", { name: "Replay" })).not.toBeDisabled());
  fireEvent.click(screen.getByRole("button", { name: "Replay" }));
  expect(screen.getByRole("tab", { name: "Workers" })).toBeDisabled();
  expect(screen.getByText("50,000 active sessions")).toBeInTheDocument();
  mockOrg = "org_b";
  (getMonitoring as jest.Mock).mockResolvedValue(snapshot("2026-10-14T12:00:02Z", 99));
  rendered.rerender(
    <QueryClientProvider client={client}>
      <MonitoringPage scope="fleet" />
    </QueryClientProvider>,
  );
  await screen.findByText("99 active sessions");
  expect(screen.getByRole("tab", { name: "Workers" })).not.toBeDisabled();
  expect(screen.queryByText("50,000 active sessions")).not.toBeInTheDocument();
});

test("an authorization failure hides previously cached fleet data", async () => {
  const { client, element } = app();
  render(element);
  await screen.findByText("50,000 active sessions");
  (getMonitoring as jest.Mock).mockRejectedValue(
    new ApiError(403, "Forbidden", "Platform access required"),
  );
  await act(async () => {
    await client.invalidateQueries({ queryKey: ["monitoring", "fleet", "org_a"] });
  });
  await waitFor(() => expect(screen.getByText("Monitoring is unavailable")).toBeInTheDocument());
  expect(screen.queryByText("50,000 active sessions")).not.toBeInTheDocument();
});

test("lease expiry is distinguished from observed loss and a stale countdown", () => {
  const at = "2026-10-14T12:00:00Z";
  const detail: MonitoringActorDetail = {
    generated_at: at,
    actor: {
      session_id: "session_test",
      org_id: "org_a",
      session_status: "active",
      phase: "model",
      phase_since: at,
      lease_state: "held",
      holder: "worker-a",
      fence: 42,
      lease_expires_at: at,
      lease_renewed_at: at,
      lease_ttl_ms: 10000,
      runnable_since: at,
      runnable_age_ms: null,
      inbox_seq: 1284,
      consumed_seq: 1281,
      next_input_seq: 1282,
      attempts_since_release: 2,
      cancel_requested_at: null,
    },
    ownership_history: [],
    steps: [],
  };
  const rendered = render(
    <ActorDetail detail={detail} serverNow={Date.parse(at) + 1000} stale={false} />,
  );
  expect(screen.getByText("Expiry deadline passed; awaiting observation")).toBeInTheDocument();
  rendered.rerender(<ActorDetail detail={detail} serverNow={Date.parse(at) + 1000} stale={true} />);
  expect(screen.getByText("Snapshot stale")).toBeInTheDocument();
  expect(screen.getByText("held")).toBeInTheDocument();
});
