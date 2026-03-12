// SSE reconnection with exponential backoff
//
// Native EventSource handles basic reconnection, but its retry behavior
// is opaque and doesn't support backoff. This utility wraps EventSource
// lifecycle with configurable exponential backoff, max retries, and
// backoff reset on successful connection.
//
// Heartbeat-based idle detection is NOT possible with native EventSource
// (SSE comments are invisible to the JS API). If we need that in the
// future, we'd switch to fetch-based SSE parsing.

export interface ReconnectState {
  /** Number of consecutive failed reconnection attempts */
  attempt: number;
  /** Whether we've exhausted max retries */
  exhausted: boolean;
}

export interface ReconnectConfig {
  /** Initial backoff delay in ms (default: 1000) */
  initialDelayMs?: number;
  /** Maximum backoff delay in ms (default: 30000) */
  maxDelayMs?: number;
  /** Maximum consecutive error retries before giving up (default: 10) */
  maxRetries?: number;
  /** Jitter factor 0-1 to randomize delay (default: 0.2) */
  jitter?: number;
}

const DEFAULTS: Required<ReconnectConfig> = {
  initialDelayMs: 1000,
  maxDelayMs: 30_000,
  maxRetries: 10,
  jitter: 0.2,
};

/**
 * Stateful reconnection tracker. Create one per SSE connection lifecycle.
 * Call `onError()` for unexpected disconnects, `onGraceful(retryMs)` for
 * server-initiated disconnects, and `reset()` on successful connection.
 */
export function createReconnectTracker(config?: ReconnectConfig) {
  const cfg = { ...DEFAULTS, ...config };
  let attempt = 0;

  return {
    /** Call when connection is established successfully. Resets backoff. */
    reset() {
      attempt = 0;
    },

    /** Current state (for UI display). */
    get state(): ReconnectState {
      return { attempt, exhausted: attempt >= cfg.maxRetries };
    },

    /**
     * Compute delay for an unexpected error reconnect.
     * Returns delay in ms, or null if max retries exhausted.
     */
    onError(): number | null {
      if (attempt >= cfg.maxRetries) return null;
      const base = Math.min(cfg.initialDelayMs * 2 ** attempt, cfg.maxDelayMs);
      const jitterRange = base * cfg.jitter;
      const delay = base + (Math.random() * 2 - 1) * jitterRange;
      attempt++;
      return Math.round(delay);
    },

    /**
     * Compute delay for a graceful server-initiated disconnect.
     * Uses server hint, does NOT count against max retries.
     */
    onGraceful(serverRetryMs?: number): number {
      // Don't increment attempt — graceful disconnects are expected
      return serverRetryMs ?? cfg.initialDelayMs;
    },

    /** Force reset for manual retry after exhaustion. */
    forceReset() {
      attempt = 0;
    },
  };
}
