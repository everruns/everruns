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
  /** Explicit tenant/account-cache boundary for session stream sharing. */
  shareScope?: string;
  /** Snapshot consumers refetch on connected and do not need historical replay. */
  snapshotOnly?: boolean;
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
  clearSharedEventStreams();
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
  if (!options?.shareScope) return activeFactory(url, options);
  const parsed = new URL(url, window.location.origin);
  if (!/\/sessions\/[^/]+\/sse$/.test(parsed.pathname)) return activeFactory(url, options);
  const base = new URL(parsed);
  base.searchParams.delete("since_id");
  base.searchParams.delete("after_sequence");
  const key = JSON.stringify([options.shareScope, base.href, options.withCredentials ?? true]);
  const candidates = sharedStreams.get(key) ?? new Set<SharedStream>();
  // Transcript subscribers need their own replay cursor. A snapshot-only
  // stream can be promoted, but two transcript cursors are never conflated.
  let entry = [...candidates].find((candidate) => options.snapshotOnly || !candidate.transcript);
  if (!entry) {
    entry = new SharedStream(url, options);
    candidates.add(entry);
    sharedStreams.set(key, candidates);
  } else if (!options.snapshotOnly) {
    entry.promote(url);
  }
  const selected = entry;
  return selected.acquire(!options.snapshotOnly, () => {
    if (selected.size) return;
    selected.dispose();
    candidates.delete(selected);
    if (!candidates.size) sharedStreams.delete(key);
  });
}

const sharedStreams = new Map<string, Set<SharedStream>>();
const cacheIds = new WeakMap<object, number>();
let nextCacheId = 0;

export function eventStreamScope(cache: object, org: string): string {
  if (!cacheIds.has(cache)) cacheIds.set(cache, ++nextCacheId);
  return JSON.stringify([cacheIds.get(cache), org]);
}

export function clearSharedEventStreams(): void {
  for (const entries of sharedStreams.values()) for (const entry of entries) entry.dispose();
  sharedStreams.clear();
}

class SharedStream {
  private options: EventStreamOptions;
  private disposed = false;
  private source: EventStreamLike;
  private clients = new Set<
    EventStreamLike & { listeners: Map<string, Set<(event: MessageEvent) => void>> }
  >();
  private types = new Set<string>();
  private connected: MessageEvent | null = null;
  transcript = false;

  constructor(url: string, options: EventStreamOptions) {
    this.options = options;
    this.source = this.open(url);
  }
  get size() {
    return this.clients.size;
  }
  private open(url: string) {
    // Embedders still own authentication. Sharing changes no wire headers.
    const source = activeFactory(url, { withCredentials: this.options.withCredentials });
    source.onopen = (event) => {
      if (!this.disposed && source === this.source)
        for (const client of [...this.clients]) client.onopen?.(event);
    };
    source.onerror = (event) => {
      if (!this.disposed && source === this.source)
        for (const client of [...this.clients]) client.onerror?.(event);
    };
    return source;
  }
  private listen(type: string) {
    const source = this.source;
    source.addEventListener(type, (event) => {
      if (this.disposed || source !== this.source) return;
      if (type === "connected") this.connected = event;
      for (const client of [...this.clients]) {
        for (const listener of [...(client.listeners.get(type) ?? [])]) listener(event);
      }
    });
  }
  promote(url: string) {
    this.source.close();
    this.connected = null;
    this.source = this.open(url);
    for (const type of this.types) this.listen(type);
  }
  acquire(transcript: boolean, release: () => void): EventStreamLike {
    if (transcript) this.transcript = true;
    let closed = false;
    const client: EventStreamLike & { listeners: Map<string, Set<(event: MessageEvent) => void>> } =
      {
        listeners: new Map(),
        onopen: null,
        onerror: null,
        addEventListener: (type, listener) => {
          const listeners = client.listeners.get(type) ?? new Set();
          listeners.add(listener);
          client.listeners.set(type, listeners);
          if (!this.types.has(type)) {
            this.types.add(type);
            this.listen(type);
          }
          const connected = this.connected;
          if (type === "connected" && connected)
            queueMicrotask(() => {
              if (
                !this.disposed &&
                this.connected === connected &&
                !closed &&
                listeners.has(listener)
              )
                listener(connected);
            });
        },
        removeEventListener: (type, listener) => {
          client.listeners.get(type)?.delete(listener);
        },
        close: () => {
          if (closed) return;
          closed = true;
          this.clients.delete(client);
          client.listeners.clear();
          if (transcript) this.transcript = false;
          release();
        },
      };
    this.clients.add(client);
    return client;
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.source.close();
  }
}
