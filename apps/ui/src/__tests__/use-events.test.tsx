import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { useEvents } from "@/hooks/use-sessions";

// Capture SSE listeners the hook registers so the test can push events.
const mockSseListeners: Record<string, Array<(e: MessageEvent) => void>> = {};

jest.mock("@/lib/event-stream", () => ({
  createEventStream: jest.fn(() => ({
    addEventListener: (type: string, listener: (e: MessageEvent) => void) => {
      (mockSseListeners[type] ||= []).push(listener);
    },
    removeEventListener: () => {},
    close: jest.fn(),
    onopen: null,
    onerror: null,
  })),
}));

// Initial REST load returns nothing so the test exercises only the SSE path.
jest.mock("@/lib/api/events", () => ({
  listEventsPaginated: jest.fn(async () => ({ events: [], totalNonDeltaCount: 0 })),
  getSseUrl: jest.fn(() => "/sse"),
  DEFAULT_EXCLUDED_EVENTS: [],
}));

const { getSseUrl } = jest.requireMock("@/lib/api/events") as {
  getSseUrl: jest.Mock;
};

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org-1" } }),
}));

function fireSse(event: { id: string; type?: string }) {
  for (const handler of mockSseListeners["input.message"] ?? []) {
    handler({ data: JSON.stringify({ type: "input.message", ...event }) } as MessageEvent);
  }
}

describe("useEvents SSE handling", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    for (const key of Object.keys(mockSseListeners)) delete mockSseListeners[key];
    getSseUrl.mockClear();
  });

  async function mountAndConnect() {
    const view = renderHook(() => useEvents("session-1"), { wrapper });
    // REST resolves (empty) -> restLoaded -> SSE connects and registers listeners.
    await waitFor(() => expect(mockSseListeners["input.message"]?.length).toBeGreaterThan(0));
    return view;
  }

  // Regression: an empty snapshot used to open the stream with no cursor, so
  // events written before the subscription (the first message of a fresh chat)
  // were never delivered and the optimistic bubble for them stayed pinned at the
  // bottom of the transcript.
  it("asks for a full replay when the initial snapshot is empty", async () => {
    await mountAndConnect();

    expect(getSseUrl).toHaveBeenCalledWith("session-1", { afterSequence: 0 });
  });

  it("resumes from the last received event once one is known", async () => {
    await mountAndConnect();
    act(() => {
      fireSse({ id: "e1" });
    });

    // Reconnecting now must resume from e1, not replay the session again.
    getSseUrl.mockClear();
    act(() => {
      for (const handler of mockSseListeners["disconnecting"] ?? []) {
        handler({ data: JSON.stringify({ retry_ms: 0 }) } as MessageEvent);
      }
    });

    await waitFor(() => expect(getSseUrl).toHaveBeenCalledWith("session-1", { sinceId: "e1" }));
  });

  it("deduplicates events with the same id", async () => {
    const { result } = await mountAndConnect();

    act(() => {
      fireSse({ id: "e1" });
      fireSse({ id: "e1" });
    });

    expect(result.current.data).toHaveLength(1);
  });

  it("bounds the in-memory buffer at MAX_EVENTS_IN_MEMORY", async () => {
    const { result } = await mountAndConnect();

    act(() => {
      for (let i = 0; i < 5_001; i++) fireSse({ id: `e${i}` });
    });

    expect(result.current.data).toHaveLength(5_000);
  });

  it("keeps the dedup set in sync after trimming (no duplicate re-append)", async () => {
    const { result } = await mountAndConnect();

    act(() => {
      for (let i = 0; i < 5_001; i++) fireSse({ id: `e${i}` });
    });
    expect(result.current.data).toHaveLength(5_000);

    // Re-deliver an event that is still inside the in-memory window. If the
    // dedup set had been corrupted by an impure updater, this would re-append.
    const stillPresentId = result.current.data[result.current.data.length - 1].id;
    act(() => {
      fireSse({ id: stillPresentId });
    });

    expect(result.current.data).toHaveLength(5_000);
  });
});
