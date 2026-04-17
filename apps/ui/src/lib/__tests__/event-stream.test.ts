import {
  createEventStream,
  getEventStreamFactory,
  setEventStreamFactory,
  type EventStreamFactory,
  type EventStreamLike,
} from "@/lib/event-stream";

describe("event-stream factory", () => {
  let originalFactory: EventStreamFactory;

  beforeEach(() => {
    originalFactory = getEventStreamFactory();
  });

  afterEach(() => {
    setEventStreamFactory(originalFactory);
  });

  it("default factory uses native EventSource with withCredentials=true", () => {
    const calls: Array<{ url: string; init: EventSourceInit | undefined }> = [];
    const originalEventSource = globalThis.EventSource;
    globalThis.EventSource = function (
      this: EventSource,
      url: string | URL,
      init?: EventSourceInit,
    ) {
      calls.push({ url: String(url), init });
      return {
        addEventListener: () => {},
        close: () => {},
        onopen: null,
        onerror: null,
      } as unknown as EventSource;
    } as unknown as typeof EventSource;

    try {
      createEventStream("/api/events");
      expect(calls).toHaveLength(1);
      expect(calls[0].url).toBe("/api/events");
      expect(calls[0].init).toEqual({ withCredentials: true });
    } finally {
      globalThis.EventSource = originalEventSource;
    }
  });

  it("default factory forwards withCredentials=false", () => {
    const calls: Array<{ url: string; init: EventSourceInit | undefined }> = [];
    const originalEventSource = globalThis.EventSource;
    globalThis.EventSource = function (
      this: EventSource,
      url: string | URL,
      init?: EventSourceInit,
    ) {
      calls.push({ url: String(url), init });
      return {
        addEventListener: () => {},
        close: () => {},
        onopen: null,
        onerror: null,
      } as unknown as EventSource;
    } as unknown as typeof EventSource;

    try {
      createEventStream("/api/events", { withCredentials: false });
      expect(calls[0].init).toEqual({ withCredentials: false });
    } finally {
      globalThis.EventSource = originalEventSource;
    }
  });

  it("setEventStreamFactory swaps the active factory", () => {
    const fakeStream: EventStreamLike = {
      addEventListener: () => {},
      close: () => {},
      onopen: null,
      onerror: null,
    };
    const customCalls: Array<string> = [];
    setEventStreamFactory((url) => {
      customCalls.push(url);
      return fakeStream;
    });

    const result = createEventStream("/api/notifications/sse?since=abc");
    expect(result).toBe(fakeStream);
    expect(customCalls).toEqual(["/api/notifications/sse?since=abc"]);
  });

  it("setEventStreamFactory(null) restores the default", () => {
    const custom: EventStreamFactory = () =>
      ({
        addEventListener: () => {},
        close: () => {},
        onopen: null,
        onerror: null,
      }) as EventStreamLike;
    setEventStreamFactory(custom);
    expect(getEventStreamFactory()).toBe(custom);

    setEventStreamFactory(null);
    expect(getEventStreamFactory()).not.toBe(custom);
  });

  it("setEventStreamFactory returns the previous factory", () => {
    const custom: EventStreamFactory = () =>
      ({
        addEventListener: () => {},
        close: () => {},
        onopen: null,
        onerror: null,
      }) as EventStreamLike;
    const previous = setEventStreamFactory(custom);
    const restored = setEventStreamFactory(previous);
    expect(restored).toBe(custom);
  });
});
