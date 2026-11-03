// The embedded realm never receives a bearer token or uses API cookies.
// Only the parent MCP resource can proxy these requests through the host.
import { applyTheme } from "@/lib/theme";
import { setEventStreamFactory, type EventStreamLike } from "@/lib/event-stream";

export interface WorkspaceView {
  kind: string;
  id?: string;
}
export interface WorkspaceInit {
  view: WorkspaceView;
  organization_id?: string;
  context?: { theme?: string };
}
export function workspacePath(view: WorkspaceView): string {
  const sections: Record<string, string> = {
    home: "home",
    agent: "agents",
    harness: "harnesses",
    capability: "capabilities",
    knowledge: "knowledge",
    session: "sessions",
  };
  const section = sections[view.kind];
  if (!section || (view.kind !== "home" && !view.id)) throw new Error("Invalid workspace view");
  return `/mcp-app/${section}${view.id ? `/${encodeURIComponent(view.id)}` : ""}${view.kind === "session" ? "/trace" : ""}`;
}
interface Reply {
  type: string;
  id?: number;
  result?: unknown;
  error?: string;
}
let nextId = 1;
const pending = new Map<
  number,
  { resolve: (value: unknown) => void; reject: (error: Error) => void }
>();
let organization: string | undefined;

export function workspaceRequest<T>(tool: string, args: Record<string, unknown> = {}): Promise<T> {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve: (value) => resolve(value as T), reject });
    window.parent.postMessage(
      { type: "everruns:request", id, tool, arguments: { ...args, organization_id: organization } },
      "*",
    );
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error("Workspace request timed out"));
    }, 30_000);
  });
}

export function connectWorkspace(
  onInit: (init: WorkspaceInit) => void,
  onTheme?: (theme: "light" | "dark") => void,
) {
  const receive = (event: MessageEvent<Reply & WorkspaceInit>) => {
    if (event.source !== window.parent || !event.data) return;
    const message = event.data;
    if (message.type === "everruns:response" && message.id !== undefined) {
      const item = pending.get(message.id);
      if (!item) return;
      pending.delete(message.id);
      if (message.error) item.reject(new Error(message.error));
      else item.resolve(message.result);
    }
    if (message.type === "everruns:init" && message.view) {
      if (organization !== message.organization_id) {
        for (const item of pending.values()) item.reject(new Error("Organization changed"));
        pending.clear();
      }
      organization = message.organization_id;
      onInit(message);
    }
    if (message.context?.theme === "light" || message.context?.theme === "dark") {
      applyTheme(message.context.theme);
      onTheme?.(message.context.theme);
    }
  };
  window.addEventListener("message", receive);
  window.parent.postMessage({ type: "everruns:ready" }, "*");
  return () => {
    window.removeEventListener("message", receive);
    for (const item of pending.values()) item.reject(new Error("Workspace closed"));
    pending.clear();
  };
}

interface ApiReply {
  status: number;
  headers: Record<string, string>;
  body: string;
}
export function installWorkspaceTransport() {
  const nativeFetch = window.fetch.bind(window);
  const fetcher: typeof fetch = async (input, init) => {
    const request =
      input instanceof Request
        ? new Request(input, init)
        : new Request(new URL(input, window.location.origin), init);
    const url = new URL(request.url);
    if (url.origin !== window.location.origin)
      throw new Error("External requests are unavailable in the MCP workspace");
    if (!url.pathname.startsWith("/api/v1/")) {
      if (url.pathname.startsWith("/api/"))
        throw new Error("This API is unavailable in the MCP workspace");
      return nativeFetch(input, init);
    }
    const path = url.pathname.slice(4) + url.search;
    const read = request.method === "GET" || request.method === "HEAD";
    const raw = read ? "" : await request.text();
    if (request.signal.aborted) throw new DOMException("Aborted", "AbortError");
    const headers: Record<string, string> = {};
    for (const name of ["everruns-change-reason", "everruns-context-revision"]) {
      const value = request.headers.get(name);
      if (value) headers[name] = value;
    }
    const result = await workspaceRequest<ApiReply>(read ? "app_read" : "app_write", {
      path,
      method: request.method,
      headers,
      ...(raw ? { body: JSON.parse(raw) } : {}),
    });
    const bytes = Uint8Array.from(atob(result.body), (character) => character.charCodeAt(0));
    return new Response(result.status === 204 ? null : bytes, {
      status: result.status,
      headers: result.headers,
    });
  };
  window.fetch = fetcher;
  // The session hooks already support a pluggable live stream. Poll bounded
  // event pages through the bridge; the UI consumes the same event objects.
  const restore = setEventStreamFactory((url) => new WorkspaceEvents(url));
  return () => {
    window.fetch = nativeFetch;
    setEventStreamFactory(restore);
  };
}

export class WorkspaceEvents implements EventStreamLike {
  onopen: ((event: unknown) => unknown) | null = null;
  onerror: ((event: unknown) => unknown) | null = null;
  private listeners = new Map<string, Set<(event: MessageEvent) => void>>();
  private timer: ReturnType<typeof setTimeout> | undefined;
  private closed = false;
  private sequence: number | undefined;
  private opened = false;
  constructor(private url: string) {
    this.timer = setTimeout(() => this.poll(), 0);
  }
  addEventListener(type: string, listener: (event: MessageEvent) => void) {
    const items = this.listeners.get(type) ?? new Set();
    items.add(listener);
    this.listeners.set(type, items);
  }
  removeEventListener(type: string, listener: (event: MessageEvent) => void) {
    this.listeners.get(type)?.delete(listener);
  }
  close() {
    this.closed = true;
    clearTimeout(this.timer);
  }
  private emit(type: string, value: unknown) {
    const event = new MessageEvent(type, { data: JSON.stringify(value) });
    this.listeners.get(type)?.forEach((listener) => listener(event));
  }
  private async poll() {
    if (this.closed) return;
    try {
      const url = new URL(this.url, window.location.origin);
      url.pathname = url.pathname.replace(/\/sse$/, "/events");
      const original = new URLSearchParams(url.search);
      url.search = "";
      if (this.sequence === undefined && original.has("since_id")) {
        const anchor = new URL(url);
        anchor.searchParams.set("around", original.get("since_id")!);
        anchor.searchParams.set("window", "1");
        const response = await fetch(anchor);
        if (!response.ok) throw new Error("Session cursor could not be read");
        const page = (await response.json()) as { data: Array<{ id: string; sequence: number }> };
        const event = page.data.find((event) => event.id === original.get("since_id"));
        if (!event) throw new Error("Session cursor is no longer available");
        this.sequence = event.sequence;
      } else if (this.sequence === undefined && !original.has("after_sequence")) {
        // Snapshot consumers start at the durable tail; their connected handler
        // refreshes the snapshot. Transcript consumers always supply a cursor.
        const tail = new URL(url);
        tail.searchParams.set("limit", "1");
        const response = await fetch(tail);
        if (!response.ok) throw new Error("Session tail could not be read");
        const page = (await response.json()) as { data: Array<{ sequence: number }> };
        this.sequence = page.data.at(-1)?.sequence ?? 0;
      }
      url.searchParams.set("limit", "100");
      for (const exclude of original.getAll("exclude")) url.searchParams.append("exclude", exclude);
      url.searchParams.set(
        "after_sequence",
        String(this.sequence ?? Number(original.get("after_sequence") ?? "0")),
      );
      const response = await fetch(url);
      if (!response.ok) throw new Error("Live session update failed");
      const page = (await response.json()) as {
        data: Array<{ id: string; sequence: number; type: string }>;
      };
      if (this.closed) return;
      if (!this.opened) {
        this.opened = true;
        this.onopen?.({});
        this.emit("connected", {});
      }
      for (const event of [...page.data].sort((a, b) => a.sequence - b.sequence)) {
        if (event.sequence > (this.sequence ?? 0)) {
          this.emit(event.type, event);
          this.emit("message", event);
          this.sequence = event.sequence;
        }
      }
      if (!this.closed && page.data.length === 100) {
        this.timer = setTimeout(() => this.poll(), 0);
        return;
      }
    } catch (error) {
      this.onerror?.(error);
    }
    if (!this.closed) this.timer = setTimeout(() => this.poll(), 1500);
  }
}
