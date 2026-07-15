// EVE-756: the cross-session Work view lists org-wide tasks and reconciles live
// `task.updated` snapshots by task_id across per-session SSE streams. These tests
// exercise the SSE reconciliation path of useOrgTasks.
import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { SessionTask } from "@/lib/api/types";
import {
  ORG_TASKS_LIST_LIMIT,
  ORG_TASKS_LIVE_SESSION_LIMIT,
  liveOrgTaskSessionIds,
  useOrgTasks,
} from "@/hooks/use-org-tasks";

// Capture SSE listeners the hook registers so the test can push events.
const mockSseListeners: Record<string, Array<(e: MessageEvent) => void>> = {};
const mockStreams: Array<{ close: jest.Mock }> = [];

jest.mock("@/lib/event-stream", () => ({
  createEventStream: jest.fn(() => {
    const stream = {
      addEventListener: (type: string, listener: (e: MessageEvent) => void) => {
        (mockSseListeners[type] ||= []).push(listener);
      },
      removeEventListener: () => {},
      close: jest.fn(),
      onopen: null,
      onerror: null,
    };
    mockStreams.push(stream);
    return stream;
  }),
}));

const initialTask: SessionTask = {
  id: "task_1",
  session_id: "session-1",
  root_session_id: "session-1",
  kind: "subagent",
  display_name: "Indexer",
  state: "running",
  attempt: 1,
  wake_policy: "always",
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
} as unknown as SessionTask;

const mockListOrgTasks = jest.fn(async (_options?: unknown) => [initialTask]);

jest.mock("@/lib/api/session-tasks", () => ({
  listOrgTasks: (options?: unknown) => mockListOrgTasks(options),
}));

jest.mock("@/lib/api/events", () => ({
  getSseUrl: jest.fn(() => "/sse"),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" }, isLoading: false }),
}));

function fireTaskEvent(type: string, task: SessionTask) {
  for (const handler of mockSseListeners[type] ?? []) {
    handler({ data: JSON.stringify({ data: { task } }) } as MessageEvent);
  }
}

describe("useOrgTasks SSE reconciliation", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    jest.clearAllMocks();
    queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    mockStreams.length = 0;
    for (const key of Object.keys(mockSseListeners)) delete mockSseListeners[key];
  });

  async function mountAndConnect() {
    const view = renderHook(() => useOrgTasks(), { wrapper });
    // Snapshot resolves -> owning sessions derived -> SSE subscribes per session.
    await waitFor(() => expect(view.result.current.data).toHaveLength(1));
    await waitFor(() => expect(mockSseListeners["task.updated"]?.length).toBeGreaterThan(0));
    return view;
  }

  it("patches a task in place by id on task.updated without a refetch", async () => {
    const { result } = await mountAndConnect();
    expect(result.current.data?.[0].state_detail).toBeUndefined();

    act(() => {
      fireTaskEvent("task.updated", {
        ...initialTask,
        state_detail: "Embedding batch 3/5",
      } as SessionTask);
    });

    // react-query notifies observers asynchronously (batched scheduler); the
    // cache is patched synchronously, the re-render follows.
    await waitFor(() => expect(result.current.data?.[0].state_detail).toBe("Embedding batch 3/5"));
    expect(result.current.data).toHaveLength(1);
    // No second snapshot fetch was needed to reflect the delta.
    expect(mockListOrgTasks).toHaveBeenCalledTimes(1);
    expect(mockListOrgTasks).toHaveBeenCalledWith({ limit: ORG_TASKS_LIST_LIMIT });
  });

  it("appends a newly created task from a task.created snapshot", async () => {
    const { result } = await mountAndConnect();

    act(() => {
      fireTaskEvent("task.created", {
        ...initialTask,
        id: "task_2",
        display_name: "Reporter",
      } as SessionTask);
    });

    await waitFor(() => expect(result.current.data).toHaveLength(2));
    expect(result.current.data?.map((t) => t.id)).toEqual(["task_1", "task_2"]);
  });

  describe("subscription limits", () => {
    it("keeps only a capped newest-session live window", () => {
      const tasks = Array.from({ length: ORG_TASKS_LIVE_SESSION_LIMIT + 5 }, (_, index) => ({
        ...initialTask,
        id: `task_${index}`,
        session_id: `session-${String(index).padStart(2, "0")}`,
      })) as SessionTask[];

      expect(liveOrgTaskSessionIds(tasks)).toHaveLength(ORG_TASKS_LIVE_SESSION_LIMIT);
      expect(liveOrgTaskSessionIds(tasks)).not.toContain(
        `session-${String(ORG_TASKS_LIVE_SESSION_LIMIT + 1).padStart(2, "0")}`,
      );
    });

    it("opens at most the capped number of session streams for a large snapshot", async () => {
      mockListOrgTasks.mockResolvedValueOnce(
        Array.from({ length: ORG_TASKS_LIVE_SESSION_LIMIT + 10 }, (_, index) => ({
          ...initialTask,
          id: `task_${index}`,
          session_id: `session-${index}`,
        })) as SessionTask[],
      );

      renderHook(() => useOrgTasks(), { wrapper });

      await waitFor(() => expect(mockStreams).toHaveLength(ORG_TASKS_LIVE_SESSION_LIMIT));
    });

    it("coalesces connected invalidations from multiple session streams", async () => {
      const invalidateQueries = jest.spyOn(queryClient, "invalidateQueries");
      mockListOrgTasks.mockResolvedValueOnce(
        Array.from({ length: 3 }, (_, index) => ({
          ...initialTask,
          id: `task_${index}`,
          session_id: `session-${index}`,
        })) as SessionTask[],
      );

      renderHook(() => useOrgTasks(), { wrapper });
      await waitFor(() => expect(mockSseListeners.connected).toHaveLength(3));

      act(() => {
        for (const handler of mockSseListeners.connected ?? []) {
          handler({ data: "{}" } as MessageEvent);
        }
      });

      await waitFor(() => expect(invalidateQueries).toHaveBeenCalledTimes(1));
    });
  });
});
