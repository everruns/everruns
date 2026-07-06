// Minimal AG-UI client for the isolated Public Chat web app.
//
// The Public Chat surface talks only to the app-scoped public endpoints
// (`/api/v1/apps/{appId}/public-chat`), never to the console session APIs, so
// it stays isolated from the rest of Everruns. The server speaks the AG-UI wire
// protocol (RunAgentInput in, AG-UI SSE out); this module builds the request
// and parses the SSE stream into simple message callbacks.

export interface PublicChatBootstrap {
  app_id: string;
  name: string;
  logo_url?: string;
  primary_color?: string;
  welcome_message?: string;
  anonymous: boolean;
  sign_in?: { mode: string; google_client_id?: string };
  captcha?: { provider: string; site_key: string };
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
}

function endpointBase(appId: string): string {
  return `/api/v1/apps/${encodeURIComponent(appId)}/public-chat`;
}

/** Fetch the non-secret bootstrap config (branding, sign-in, captcha). */
export async function fetchBootstrap(appId: string): Promise<PublicChatBootstrap> {
  const res = await fetch(`${endpointBase(appId)}/config`, {
    headers: { Accept: "application/json" },
  });
  if (!res.ok) {
    throw new Error(`Failed to load chat (${res.status})`);
  }
  return (await res.json()) as PublicChatBootstrap;
}

export interface RunCallbacks {
  /** Full server-side message snapshot emitted at stream start. */
  onSnapshot?(messages: ChatMessage[]): void;
  onAssistantStart?(id: string): void;
  onAssistantDelta?(id: string, delta: string): void;
  onAssistantEnd?(id: string): void;
  /** Sanitized, user-facing error string. */
  onError?(message: string): void;
  onFinish?(): void;
}

export interface RunOptions {
  appId: string;
  threadId: string;
  /** Full conversation; the last entry must be the new user message. */
  messages: ChatMessage[];
  turnstileToken?: string;
  bearerToken?: string;
  signal?: AbortSignal;
  callbacks: RunCallbacks;
}

/** A friendly fallback message for any non-OK HTTP status before streaming. */
function statusMessage(status: number): string {
  if (status === 401 || status === 403) return "You don't have access to this chat.";
  if (status === 404) return "This chat is not available.";
  if (status === 429) return "You're sending messages too quickly. Please wait a moment.";
  if (status === 503) return "The chat is temporarily unavailable. Please try again shortly.";
  return "Something went wrong. Please try again.";
}

/**
 * Run one turn and stream the AG-UI response. Resolves when the stream ends.
 * Errors are reported via `callbacks.onError` rather than thrown so the caller
 * can keep the conversation usable.
 */
export async function runPublicChat(options: RunOptions): Promise<void> {
  const { appId, threadId, messages, turnstileToken, bearerToken, signal, callbacks } = options;

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "text/event-stream",
  };
  if (turnstileToken) headers["x-everruns-turnstile-token"] = turnstileToken;
  if (bearerToken) headers["Authorization"] = `Bearer ${bearerToken}`;

  const body = {
    threadId,
    runId: crypto.randomUUID(),
    state: {},
    messages: messages.map((m) => ({ role: m.role, id: m.id, content: m.content })),
    tools: [],
    context: [],
    forwardedProps: {},
  };

  let res: Response;
  try {
    res = await fetch(endpointBase(appId), {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal,
    });
  } catch {
    callbacks.onError?.("Couldn't reach the chat. Check your connection and try again.");
    return;
  }

  if (!res.ok || !res.body) {
    callbacks.onError?.(statusMessage(res.status));
    return;
  }

  await parseSseStream(res.body, callbacks, signal);
}

/** Parse an AG-UI SSE byte stream and dispatch events to callbacks. */
async function parseSseStream(
  body: ReadableStream<Uint8Array>,
  callbacks: RunCallbacks,
  signal?: AbortSignal,
): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  try {
    for (;;) {
      if (signal?.aborted) break;
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });

      // SSE events are separated by a blank line.
      let sep: number;
      while ((sep = buffer.indexOf("\n\n")) !== -1) {
        const rawEvent = buffer.slice(0, sep);
        buffer = buffer.slice(sep + 2);
        dispatchEvent(rawEvent, callbacks);
      }
    }
  } catch {
    // Network drop mid-stream — surface a soft error; partial content stays.
    callbacks.onError?.("The connection was interrupted. Please try again.");
  } finally {
    reader.releaseLock();
  }
}

function dispatchEvent(rawEvent: string, callbacks: RunCallbacks): void {
  // Collect `data:` payload lines (ignore comments / keepalive lines).
  const dataLines = rawEvent
    .split("\n")
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice(5).trimStart());
  if (dataLines.length === 0) return;

  let event: Record<string, unknown>;
  try {
    event = JSON.parse(dataLines.join("\n")) as Record<string, unknown>;
  } catch {
    return;
  }

  switch (event.type) {
    case "MESSAGES_SNAPSHOT": {
      const raw = Array.isArray(event.messages) ? (event.messages as unknown[]) : [];
      const snapshot: ChatMessage[] = [];
      for (const entry of raw) {
        const m = entry as { role?: string; id?: string; content?: string };
        if ((m.role === "user" || m.role === "assistant") && typeof m.id === "string") {
          snapshot.push({ id: m.id, role: m.role, content: m.content ?? "" });
        }
      }
      callbacks.onSnapshot?.(snapshot);
      break;
    }
    case "TEXT_MESSAGE_START":
      if (typeof event.messageId === "string") callbacks.onAssistantStart?.(event.messageId);
      break;
    case "TEXT_MESSAGE_CONTENT":
      if (typeof event.messageId === "string" && typeof event.delta === "string") {
        callbacks.onAssistantDelta?.(event.messageId, event.delta);
      }
      break;
    case "TEXT_MESSAGE_END":
      if (typeof event.messageId === "string") callbacks.onAssistantEnd?.(event.messageId);
      break;
    case "RUN_ERROR":
      callbacks.onError?.(
        typeof event.message === "string" && event.message
          ? event.message
          : "Something went wrong. Please try again.",
      );
      break;
    case "RUN_FINISHED":
      callbacks.onFinish?.();
      break;
    default:
      // Thinking / tool-activity events are ignored on the public surface.
      break;
  }
}
