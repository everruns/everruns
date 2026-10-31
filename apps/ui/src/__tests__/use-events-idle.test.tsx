import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useEvents } from "@/hooks/use-sessions";
import { withSessionActivity } from "@/lib/session-activity";
import { api } from "@/lib/api/client";
import { createMessage } from "@/lib/api/messages";
import {
  submitToolApprovals,
  submitToolResults,
  submitQuestionAnswers,
  submitElicitationConsent,
} from "@/lib/api/sessions";
import type { Event } from "@/lib/api/types";

type Stream = {
  listeners: Record<string, Array<(e: MessageEvent) => void>>;
  close: jest.Mock;
  onerror: (() => void) | null;
};
const mockStreams: Stream[] = [];
let mockSnapshot: Event[] = [];
let mockTail: Event[] = [];
let mockStatus = "idle";
let mockTasks: Array<{ state: string }> = [];
jest.mock("@/lib/event-stream", () => ({
  eventStreamScope: jest.requireActual("@/lib/event-stream").eventStreamScope,
  createEventStream: jest.fn(() => {
    const stream = {
      listeners: {} as Stream["listeners"],
      close: jest.fn(),
      onopen: null,
      onerror: null,
      addEventListener(type: string, listener: (e: MessageEvent) => void) {
        (this.listeners[type] ||= []).push(listener);
      },
    };
    mockStreams.push(stream);
    return stream;
  }),
}));
jest.mock("@/lib/api/client", () => ({ api: { get: jest.fn(), post: jest.fn() } }));
jest.mock("@/lib/api/events", () => ({
  listEventsPaginated: jest.fn(async (_id, options) => ({
    events: options.limit === 1 ? mockTail : mockSnapshot,
    totalNonDeltaCount: mockSnapshot.length,
  })),
  getSseUrl: jest.fn(() => "/sse"),
  DEFAULT_EXCLUDED_EVENTS: [],
}));
const { getSseUrl, listEventsPaginated } = jest.requireMock("@/lib/api/events");
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" } }),
}));
function latest() {
  return mockStreams[mockStreams.length - 1];
}
function emit(type: string, data: unknown, stream = latest()) {
  for (const listener of stream.listeners[type] ?? [])
    listener({ data: JSON.stringify(data) } as MessageEvent);
}
function event(id: string, type = "input.message", sequence?: number): Event {
  return { id, type, sequence } as Event;
}
function push(value: Event, stream = latest()) {
  if (value.sequence != null) mockTail = [value];
  emit(value.type, value, stream);
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("optional idle SSE lifecycle", () => {
  let client: QueryClient;
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  beforeEach(() => {
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    mockStreams.length = 0;
    mockSnapshot = [];
    mockTail = [];
    mockStatus = "idle";
    mockTasks = [];
    jest.clearAllMocks();
    (api.get as jest.Mock).mockImplementation(async (url: string) => ({
      data: url.endsWith("/tasks") ? mockTasks : { status: mockStatus },
    }));
    (api.post as jest.Mock).mockResolvedValue({ data: {} });
  });
  async function mount(followIdle = false) {
    const view = renderHook(({ id, enabled }) => useEvents(id, { followIdle, enabled }), {
      initialProps: { id: "session-1", enabled: true },
      wrapper,
    });
    await waitFor(() => expect(mockStreams).toHaveLength(1));
    act(() => emit("connected", {}));
    return view;
  }
  it("keeps continuous following for external turns without idle probes", async () => {
    const { result } = await mount(true);
    act(() => {
      push(event("idle", "session.idled", 1));
      push(event("external", "turn.started", 2));
    });
    expect(latest().close).not.toHaveBeenCalled();
    expect(api.get).not.toHaveBeenCalled();
    expect(result.current.data.map((e) => e.id)).toEqual(["idle", "external"]);
  });
  it("suspends a fresh session and explicitly resumes from zero", async () => {
    mockStatus = "started";
    const { result } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    expect(result.current.isReconnecting).toBe(false);
    act(() => result.current.resume());
    expect(getSseUrl).toHaveBeenLastCalledWith("session-1", { afterSequence: 0 });
    expect(result.current.isSuspended).toBe(false);
  });
  it("reopens on send from the durable cursor with duplicate-free replay", async () => {
    mockSnapshot = mockTail = [event("idle", "session.idled", 1)];
    const { result } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    const request = deferred<{ data: object }>();
    (api.post as jest.Mock).mockReturnValueOnce(request.promise);
    let send!: Promise<unknown>;
    act(() => {
      send = createMessage("session-1", { message: { role: "user", content: [] } });
    });
    expect(getSseUrl).toHaveBeenLastCalledWith("session-1", { sinceId: "idle" });
    act(() => {
      emit("connected", {});
      push(event("idle", "session.idled", 1));
      push(event("input", "input.message", 2));
      push(event("delta", "output.message.delta"));
    });
    await act(async () => {
      request.resolve({ data: {} });
      await send;
    });
    expect(result.current.isSuspended).toBe(false);
    act(() => push(event("done", "session.idled", 3)));
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    expect(result.current.data.map((e) => e.id)).toEqual(["idle", "input", "delta", "done"]);
    act(() => result.current.resume());
    expect(getSseUrl).toHaveBeenLastCalledWith("session-1", { sinceId: "done" });
  });
  it("does not cut off catch-up at a historical idled event", async () => {
    mockSnapshot = [event("old-idle", "session.idled", 1)];
    mockTail = [event("new-idle", "session.idled", 4)];
    const { result } = await mount();
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    act(() => {
      push(event("input", "input.message", 2));
      push(event("started", "turn.started", 3));
      push(event("new-idle", "session.idled", 4));
    });
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
  });
  it("invalidates in-flight idle checks when new work arrives", async () => {
    const status = deferred<{ data: object }>();
    (api.get as jest.Mock).mockReturnValueOnce(status.promise);
    const { result } = await mount();
    act(() => push(event("new-input", "input.message", 1)));
    await act(async () => status.resolve({ data: { status: "idle" } }));
    expect(result.current.isSuspended).toBe(false);
    expect(latest().close).not.toHaveBeenCalled();
  });
  it.each(["active", "waiting_for_tool_results"])(
    "keeps %s work live even with an empty window",
    async (status) => {
      mockStatus = status;
      const { result } = await mount();
      await act(async () => {});
      expect(result.current.isSuspended).toBe(false);
    },
  );
  it.each(["setup_connection", "approve_tool_call", "ask_user", "confirm_url_elicitation"])(
    "keeps a pending %s live",
    async (name) => {
      mockSnapshot = mockTail = [
        { ...event("pending", "tool.call_requested", 1), data: { tool_name: name } } as Event,
      ];
      mockStatus = "waiting_for_tool_results";
      const { result } = await mount();
      await act(async () => {});
      expect(result.current.isSuspended).toBe(false);
    },
  );
  it.each(["queued", "running", "awaiting_input"])(
    "keeps %s tasks live until terminal",
    async (state) => {
      mockTasks = [{ state }];
      const { result } = await mount();
      await act(async () => {});
      expect(result.current.isSuspended).toBe(false);
      mockTasks = [{ state: "succeeded" }];
      act(() => push(event("task-done", "task.updated", 1)));
      await waitFor(() => expect(result.current.isSuspended).toBe(true));
    },
  );
  it.each([
    ["approval", () => submitToolApprovals("session-1", [])],
    ["resume", () => submitToolResults("session-1", [])],
    ["answer", () => submitQuestionAnswers("session-1", { tool_call_id: "call", answers: [] })],
    ["consent", () => submitElicitationConsent("session-1", "call", "accept")],
  ] as const)("holds the stream throughout a %s request", async (_name, action) => {
    const { result } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    const request = deferred<{ data: object }>();
    (api.post as jest.Mock).mockReturnValueOnce(request.promise);
    let pending!: Promise<unknown>;
    act(() => {
      pending = action();
      emit("connected", {});
    });
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    act(() => push(event("done", "session.idled", 1)));
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    await act(async () => {
      request.resolve({ data: {} });
      await pending;
    });
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
  });
  it("fails open when verification is unavailable", async () => {
    (api.get as jest.Mock).mockRejectedValueOnce(new Error("offline"));
    const { result } = await mount();
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
  });
  it.each(["focus", "online"])("recovers external events on %s", async (type) => {
    const { result } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    mockTail = [event("outside", "input.message", 1)];
    act(() => {
      window.dispatchEvent(new window.Event(type));
      emit("connected", {});
    });
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    act(() => push(event("outside", "input.message", 1)));
    expect(result.current.data.map((e) => e.id)).toEqual(["outside"]);
  });
  it("retries errors from the durable cursor and ignores stale callbacks", async () => {
    await mount(true);
    const old = latest();
    act(() => {
      push(event("durable", "input.message", 1));
      old.onerror?.();
    });
    act(() => push(event("late", "input.message", 2), old));
    await waitFor(() => expect(mockStreams).toHaveLength(2), { timeout: 3000 });
    expect(getSseUrl).toHaveBeenLastCalledWith("session-1", { sinceId: "durable" });
  });
  it.each(["unmount", "disable"])("cancels retry timers on %s", async (action) => {
    const view = await mount(true);
    act(() => emit("disconnecting", { retry_ms: 20 }));
    if (action === "unmount") view.unmount();
    else view.rerender({ id: "session-1", enabled: false });
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(mockStreams).toHaveLength(1);
  });
  it("isolates session cursors during reconnect and ignores old events", async () => {
    const { result, rerender } = await mount();
    const old = latest();
    act(() => {
      push(event("old", "input.message", 1));
      emit("disconnecting", { retry_ms: 20 });
    });
    rerender({ id: "session-2", enabled: true });
    await waitFor(() => expect(mockStreams).toHaveLength(2));
    act(() => push(event("late-old", "input.message", 2), old));
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(getSseUrl).toHaveBeenLastCalledWith("session-2", { afterSequence: 0 });
    expect(result.current.data).toEqual([]);
    expect(mockStreams).toHaveLength(2);
  });
  it("holds concurrent actions independently and releases failed requests", async () => {
    const { result } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    const first = deferred<void>();
    const second = deferred<void>();
    let one!: Promise<void>;
    let two!: Promise<void>;
    act(() => {
      one = withSessionActivity("session-1", () => first.promise);
      two = withSessionActivity("session-1", () => second.promise);
      emit("connected", {});
    });
    await act(async () => {
      first.resolve();
      await one;
    });
    expect(result.current.isSuspended).toBe(false);
    await act(async () => {
      second.resolve();
      await two;
    });
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    await act(async () => {
      await expect(
        withSessionActivity("session-1", async () => {
          throw new Error("rejected");
        }),
      ).rejects.toThrow("rejected");
    });
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
  });
  it("does not wake another session for a mutation", async () => {
    const { result } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    await act(async () => {
      await createMessage("session-other", { message: { role: "user", content: [] } });
    });
    expect(mockStreams).toHaveLength(1);
    expect(result.current.isSuspended).toBe(true);
  });
  it("ignores idle checks completed after a session switch", async () => {
    const status = deferred<{ data: object }>();
    (api.get as jest.Mock).mockReturnValueOnce(status.promise);
    const { result, rerender } = await mount();
    rerender({ id: "session-2", enabled: true });
    await waitFor(() => expect(mockStreams).toHaveLength(2));
    await act(async () => status.resolve({ data: { status: "idle" } }));
    expect(latest().close).not.toHaveBeenCalled();
    expect(result.current.isSuspended).toBe(false);
  });
  it("bounds verification concurrency during a burst of task events", async () => {
    const status = deferred<{ data: object }>();
    (api.get as jest.Mock).mockReturnValueOnce(status.promise);
    const { result } = await mount();
    act(() => {
      for (let sequence = 1; sequence <= 100; sequence++)
        push(event(`task-${sequence}`, "task.updated", sequence));
    });
    // One session read and one task read are in flight; the burst is coalesced.
    expect(api.get).toHaveBeenCalledTimes(2);
    await act(async () => status.resolve({ data: { status: "idle" } }));
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    expect(api.get).toHaveBeenCalledTimes(4);
  });
  it("reconnects opted-in active work from its cursor before suspending at settlement", async () => {
    mockStatus = "active";
    const { result } = await mount();
    act(() => {
      push(event("active", "turn.started", 1));
      emit("disconnecting", { retry_ms: 0 });
    });
    await waitFor(() => expect(mockStreams).toHaveLength(2));
    expect(getSseUrl).toHaveBeenLastCalledWith("session-1", { sinceId: "active" });
    act(() => emit("connected", {}));
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    mockStatus = "idle";
    act(() => push(event("idle", "session.idled", 2)));
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
  });
  it("ignores an initial REST snapshot completed after switching sessions", async () => {
    const snapshot = deferred<{ events: Event[] }>();
    listEventsPaginated.mockReturnValueOnce(snapshot.promise);
    const view = renderHook(({ id }) => useEvents(id, { followIdle: false }), {
      initialProps: { id: "session-1" },
      wrapper,
    });
    view.rerender({ id: "session-2" });
    await waitFor(() => expect(mockStreams).toHaveLength(1));
    await act(async () =>
      snapshot.resolve({ events: [event("old-session-idle", "session.idled", 9)] }),
    );
    expect(view.result.current.data).toEqual([]);
    expect(getSseUrl).toHaveBeenLastCalledWith("session-2", { afterSequence: 0 });
    expect(mockStreams).toHaveLength(1);
  });
  it("holds following when a mutation starts before the hook subscribes", async () => {
    const request = deferred<void>();
    const pending = withSessionActivity("session-1", () => request.promise);
    const { result } = await mount();
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    expect(mockStreams).toHaveLength(1);
    await act(async () => {
      request.resolve();
      await pending;
    });
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
  });
  it("retains mutation holds across disabling and re-enabling the hook", async () => {
    const { result, rerender } = await mount();
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
    const request = deferred<void>();
    let pending!: Promise<void>;
    act(() => {
      pending = withSessionActivity("session-1", () => request.promise);
    });
    rerender({ id: "session-1", enabled: false });
    rerender({ id: "session-1", enabled: true });
    await waitFor(() => expect(mockStreams).toHaveLength(3));
    act(() => emit("connected", {}));
    await act(async () => {});
    expect(result.current.isSuspended).toBe(false);
    await act(async () => {
      request.resolve();
      await pending;
    });
    await waitFor(() => expect(result.current.isSuspended).toBe(true));
  });
  it("preserves the loaded transcript across disabling instead of replacing it with another REST snapshot", async () => {
    const { result, rerender } = await mount(true);
    act(() => push(event("input", "input.message", 1)));
    rerender({ id: "session-1", enabled: false });
    rerender({ id: "session-1", enabled: true });
    await waitFor(() => expect(mockStreams).toHaveLength(2));
    await act(async () => {});
    expect(result.current.data.map((e) => e.id)).toEqual(["input"]);
    expect(listEventsPaginated).toHaveBeenCalledTimes(1);
    expect(getSseUrl).toHaveBeenLastCalledWith("session-1", { sinceId: "input" });
  });
  it("recovers continuous streams on online after the retry budget is exhausted, without zombie reconnects", async () => {
    const { result, unmount } = await mount(true);
    jest.useFakeTimers();
    try {
      for (let attempt = 0; attempt < 11; attempt++) {
        act(() => mockStreams.at(-1)!.onerror?.());
        act(() => jest.advanceTimersByTime(40_000));
      }
      expect(result.current.error?.message).toContain("max retries");
      const count = mockStreams.length;
      act(() => window.dispatchEvent(new window.Event("online")));
      expect(mockStreams).toHaveLength(count + 1);
      act(() => window.dispatchEvent(new window.Event("online")));
      expect(mockStreams).toHaveLength(count + 1);
      unmount();
      act(() => {
        window.dispatchEvent(new window.Event("online"));
        jest.runOnlyPendingTimers();
      });
      expect(mockStreams).toHaveLength(count + 1);
    } finally {
      jest.useRealTimers();
    }
  });
});
