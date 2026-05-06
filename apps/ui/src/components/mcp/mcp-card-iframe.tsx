"use client";

// Renders an Everruns MCP Apps card (specs/mcp-cards.md) inside a
// sandboxed iframe. The card HTML is server-rendered and arrives as the
// `text` payload of an embedded MCP resource at `ui://everruns/...`.
//
// Sandboxing follows the host requirements in the spec:
// - allow-scripts only; no allow-same-origin, no forms, no popups,
//   no top-navigation
// - srcdoc carries the HTML directly (no network fetch)
// - referrerpolicy="no-referrer"
// - postMessage events are validated against the iframe's
//   contentWindow before being dispatched
//
// Phase 1 ships read-only cards. The action protocol below is wired so
// that when card buttons start posting `tool` / `prompt` / `intent` /
// `link` / `notify` messages, the host already routes them through a
// single typed callback. The component itself never invokes tools — that
// remains the host's responsibility behind whatever permission UX the
// caller wants.

import { useEffect, useMemo, useRef } from "react";

export type CardMessage =
  | { type: "tool"; payload: { toolName: string; params?: Record<string, unknown> } }
  | { type: "prompt"; payload: { prompt: string } }
  | { type: "intent"; payload: { intent: string; params?: Record<string, unknown> } }
  | { type: "link"; payload: { url: string } }
  | { type: "notify"; payload: { message: string } };

export interface McpCardIframeProps {
  /** `ui://everruns/{entity}/{id}/card` URI from the embedded resource. */
  uri: string;
  /** Server-rendered HTML document. MIME type must be `text/html`. */
  html: string;
  /**
   * Callback invoked for each validated card message. The component does
   * not act on messages itself — the host decides which to honor and
   * applies its own auth / confirmation UX before routing to
   * `tools/call`. Phase 1 cards never emit messages.
   */
  onAction?: (msg: CardMessage) => void;
  /** Initial iframe height in px. Cards are designed for ~360px wide. */
  height?: number;
  className?: string;
  /** Optional title for the iframe (a11y). */
  title?: string;
}

/** Maximum messages per second per iframe before drops kick in. */
const MESSAGE_RATE_LIMIT = 10;

const KNOWN_TYPES: ReadonlySet<CardMessage["type"]> = new Set([
  "tool",
  "prompt",
  "intent",
  "link",
  "notify",
]);

function isCardMessage(value: unknown): value is CardMessage {
  if (typeof value !== "object" || value === null) return false;
  const v = value as { type?: unknown; payload?: unknown };
  if (typeof v.type !== "string" || !KNOWN_TYPES.has(v.type as CardMessage["type"])) {
    return false;
  }
  if (typeof v.payload !== "object" || v.payload === null) return false;
  return true;
}

export function McpCardIframe({
  uri,
  html,
  onAction,
  height = 280,
  className,
  title,
}: McpCardIframeProps) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const rateTokensRef = useRef({ tokens: MESSAGE_RATE_LIMIT, lastRefill: Date.now() });

  // Hash the URI for stable iframe identity (helps React reconcile when
  // many cards live on one page).
  const iframeName = useMemo(() => `mcp-card-${hashCode(uri)}`, [uri]);

  useEffect(() => {
    if (!onAction) return;
    const dispatch = onAction;

    function handler(event: MessageEvent) {
      const iframe = iframeRef.current;
      if (!iframe || event.source !== iframe.contentWindow) return;
      if (!isCardMessage(event.data)) return;

      // Token-bucket rate limit: refill MESSAGE_RATE_LIMIT tokens per second.
      const now = Date.now();
      const bucket = rateTokensRef.current;
      const refill = ((now - bucket.lastRefill) / 1000) * MESSAGE_RATE_LIMIT;
      bucket.tokens = Math.min(MESSAGE_RATE_LIMIT, bucket.tokens + refill);
      bucket.lastRefill = now;
      if (bucket.tokens < 1) return;
      bucket.tokens -= 1;

      dispatch(event.data);
    }

    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, [onAction]);

  return (
    <iframe
      ref={iframeRef}
      name={iframeName}
      title={title ?? `MCP card (${uri})`}
      data-mcp-card-uri={uri}
      // sandbox lacks allow-same-origin on purpose — keeps the iframe in
      // an opaque origin so it cannot read parent state, cookies, or
      // localStorage. Action dispatch only goes through postMessage.
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      srcDoc={html}
      style={{
        width: "100%",
        maxWidth: 400,
        height,
        border: "none",
        background: "transparent",
        display: "block",
      }}
      className={className}
    />
  );
}

function hashCode(s: string): string {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  return Math.abs(h).toString(36);
}
