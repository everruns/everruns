import { createReconnectTracker } from "@/lib/sse-reconnect";

describe("createReconnectTracker", () => {
  beforeEach(() => {
    jest.spyOn(Math, "random").mockReturnValue(0.5);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("returns initial state with zero attempts", () => {
    const tracker = createReconnectTracker();
    expect(tracker.state).toEqual({ attempt: 0, exhausted: false });
  });

  it("computes exponential backoff delays on error", () => {
    const tracker = createReconnectTracker({ initialDelayMs: 1000, jitter: 0 });

    expect(tracker.onError()).toBe(1000); // 1s
    expect(tracker.onError()).toBe(2000); // 2s
    expect(tracker.onError()).toBe(4000); // 4s
    expect(tracker.onError()).toBe(8000); // 8s
  });

  it("caps delay at maxDelayMs", () => {
    const tracker = createReconnectTracker({
      initialDelayMs: 1000,
      maxDelayMs: 5000,
      jitter: 0,
    });

    tracker.onError(); // 1s
    tracker.onError(); // 2s
    tracker.onError(); // 4s
    expect(tracker.onError()).toBe(5000); // capped at 5s, not 8s
  });

  it("returns null after max retries exhausted", () => {
    const tracker = createReconnectTracker({ maxRetries: 3, jitter: 0 });

    expect(tracker.onError()).not.toBeNull(); // attempt 0
    expect(tracker.onError()).not.toBeNull(); // attempt 1
    expect(tracker.onError()).not.toBeNull(); // attempt 2
    expect(tracker.onError()).toBeNull(); // exhausted

    expect(tracker.state).toEqual({ attempt: 3, exhausted: true });
  });

  it("resets backoff on successful connection", () => {
    const tracker = createReconnectTracker({ jitter: 0 });

    tracker.onError(); // attempt 0
    tracker.onError(); // attempt 1
    expect(tracker.state.attempt).toBe(2);

    tracker.reset();
    expect(tracker.state).toEqual({ attempt: 0, exhausted: false });

    // Next error starts from beginning
    expect(tracker.onError()).toBe(1000);
  });

  it("graceful disconnect uses server retry hint", () => {
    const tracker = createReconnectTracker();

    expect(tracker.onGraceful(500)).toBe(500);
    expect(tracker.onGraceful(2000)).toBe(2000);
  });

  it("graceful disconnect does not count against max retries", () => {
    const tracker = createReconnectTracker({ maxRetries: 2 });

    // Many graceful disconnects
    tracker.onGraceful(100);
    tracker.onGraceful(100);
    tracker.onGraceful(100);
    tracker.onGraceful(100);

    expect(tracker.state.attempt).toBe(0);
    expect(tracker.state.exhausted).toBe(false);

    // Error retries still available
    expect(tracker.onError()).not.toBeNull();
    expect(tracker.onError()).not.toBeNull();
  });

  it("graceful disconnect falls back to initialDelayMs when no hint", () => {
    const tracker = createReconnectTracker({ initialDelayMs: 750 });

    expect(tracker.onGraceful()).toBe(750);
    expect(tracker.onGraceful(undefined)).toBe(750);
  });

  it("applies jitter to delay", () => {
    // random=0.5 → jitter offset = base * 0.2 * (2*0.5 - 1) = 0
    jest.spyOn(Math, "random").mockReturnValue(0.5);
    const tracker1 = createReconnectTracker({ initialDelayMs: 1000, jitter: 0.2 });
    expect(tracker1.onError()).toBe(1000); // 1000 + 0

    // random=1.0 → jitter offset = 1000 * 0.2 * (2*1 - 1) = +200
    jest.spyOn(Math, "random").mockReturnValue(1.0);
    const tracker2 = createReconnectTracker({ initialDelayMs: 1000, jitter: 0.2 });
    expect(tracker2.onError()).toBe(1200);

    // random=0.0 → jitter offset = 1000 * 0.2 * (2*0 - 1) = -200
    jest.spyOn(Math, "random").mockReturnValue(0.0);
    const tracker3 = createReconnectTracker({ initialDelayMs: 1000, jitter: 0.2 });
    expect(tracker3.onError()).toBe(800);
  });

  it("forceReset allows retry after exhaustion", () => {
    const tracker = createReconnectTracker({ maxRetries: 1, jitter: 0 });

    tracker.onError(); // attempt 0
    expect(tracker.onError()).toBeNull(); // exhausted
    expect(tracker.state.exhausted).toBe(true);

    tracker.forceReset();
    expect(tracker.state).toEqual({ attempt: 0, exhausted: false });
    expect(tracker.onError()).toBe(1000); // works again
  });
});
