// Pluggable SSE transport for UI live streams.
//
// OSS UI opens live streams via native `EventSource`, which authenticates
// through cookies. Downstream embedders (e.g. SaaS with bearer tokens) may
// need an alternate transport since native `EventSource` cannot set request
// headers. Previously, downstreams had to patch `window.EventSource`
// globally. This module provides a narrow factory that downstreams can
// override in one place without forking the hooks/providers.
//
// Default behavior (OSS) is unchanged: native `EventSource` with
// `withCredentials: true` + cookies.

export interface EventStreamLike {
  addEventListener(type: string, listener: (event: MessageEvent) => void): void;
  removeEventListener?(type: string, listener: (event: MessageEvent) => void): void;
  close(): void;
  onopen: ((ev: unknown) => unknown) | null;
  onerror: ((ev: unknown) => unknown) | null;
}

export interface EventStreamOptions {
  /** Send credentials (cookies) with the request. Default: true. */
  withCredentials?: boolean;
}

export type EventStreamFactory = (url: string, options?: EventStreamOptions) => EventStreamLike;

const defaultFactory: EventStreamFactory = (url, options) =>
  new EventSource(url, {
    withCredentials: options?.withCredentials ?? true,
  }) as unknown as EventStreamLike;

let activeFactory: EventStreamFactory = defaultFactory;

/**
 * Override the global SSE transport factory. Call with `null` to restore the
 * default (native `EventSource`). Intended for downstream embedders that need
 * header-based auth or custom reconnect behavior.
 *
 * Returns the previous factory so callers can restore it if needed (useful in
 * tests).
 */
export function setEventStreamFactory(factory: EventStreamFactory | null): EventStreamFactory {
  const previous = activeFactory;
  activeFactory = factory ?? defaultFactory;
  return previous;
}

/** Return the currently active factory (primarily for tests). */
export function getEventStreamFactory(): EventStreamFactory {
  return activeFactory;
}

/**
 * Open an SSE stream using the currently active factory. Prefer this over
 * constructing `EventSource` directly so downstream embedders can inject a
 * custom authenticated transport.
 */
export function createEventStream(url: string, options?: EventStreamOptions): EventStreamLike {
  return activeFactory(url, options);
}
