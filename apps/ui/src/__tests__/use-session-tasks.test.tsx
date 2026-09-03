// EVERRUNS-1M: every useSessionTasks instance used to open its own SSE stream,
// so the header chips plus the Work tab on a couple of tabs exhausted the
// server's per-session stream cap. These tests pin the shared, ref-counted
// stream per session.
import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import type { SessionTask } from "@/lib/api/types";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import { queryKeys } from "@/lib/query-keys";

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

const { createEventStream } = jest.requireMock("@/lib/event-stream") as {
  createEventStream: jest.Mock;
};

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

jest.mock("@/lib/api/session-tasks", () => ({
  listSessionTasks: jest.fn(async () => [initialTask]),
  getSessionTask: jest.fn(),
  cancelSessionTask: jest.fn(),
  postSessionTaskMessage: jest.fn(),
}));

jest.mock("@/lib/api/events", () => ({
  getSseUrl: jest.fn((sessionId: string) => `/sessions/${sessionId}/sse`),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" }, isLoading: false }),
}));

function fireTaskEvent(type: string, task: SessionTask) {
  for (const handler of mockSseListeners[type] ?? []) {
    handler({ data: JSON.stringify({ data: { task } }) } as MessageEvent);
  }
}

describe("useSessionTasks shared SSE stream", () => {
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

  it("opens one stream per session across hook instances and closes it with the last", async () => {
    const first = renderHook(() => useSessionTasks("session-1"), { wrapper });
    const second = renderHook(() => useSessionTasks("session-1"), { wrapper });
    await waitFor(() => expect(first.result.current.data).toHaveLength(1));
    await waitFor(() => expect(second.result.current.data).toHaveLength(1));

    expect(createEventStream).toHaveBeenCalledTimes(1);
    expect(createEventStream).toHaveBeenCalledWith("/sessions/session-1/sse", {
      withCredentials: true,
    });

    first.unmount();
    expect(mockStreams[0].close).not.toHaveBeenCalled();

    second.unmount();
    expect(mockStreams[0].close).toHaveBeenCalledTimes(1);
  });

  it("opens separate streams for different sessions", async () => {
    const a = renderHook(() => useSessionTasks("session-1"), { wrapper });
    const b = renderHook(() => useSessionTasks("session-2"), { wrapper });
    await waitFor(() => expect(createEventStream).toHaveBeenCalledTimes(2));
    a.unmount();
    b.unmount();
    expect(mockStreams.every((s) => s.close.mock.calls.length === 1)).toBe(true);
  });

  it("patches task snapshots into the shared list cache", async () => {
    const view = renderHook(() => useSessionTasks("session-1"), { wrapper });
    await waitFor(() => expect(view.result.current.data).toHaveLength(1));
    await waitFor(() => expect(mockSseListeners["task.updated"]?.length).toBeGreaterThan(0));

    act(() => {
      fireTaskEvent("task.updated", { ...initialTask, state: "succeeded" });
    });

    await waitFor(() =>
      expect(
        queryClient.getQueryData<SessionTask[]>(queryKeys.sessionTasks.list("session-1"))?.[0]
          .state,
      ).toBe("succeeded"),
    );
    view.unmount();
  });

  it("reopens a stream after the last subscriber released it", async () => {
    const first = renderHook(() => useSessionTasks("session-1"), { wrapper });
    await waitFor(() => expect(createEventStream).toHaveBeenCalledTimes(1));
    first.unmount();
    expect(mockStreams[0].close).toHaveBeenCalledTimes(1);

    const second = renderHook(() => useSessionTasks("session-1"), { wrapper });
    await waitFor(() => expect(createEventStream).toHaveBeenCalledTimes(2));
    second.unmount();
    expect(mockStreams[1].close).toHaveBeenCalledTimes(1);
  });
});
