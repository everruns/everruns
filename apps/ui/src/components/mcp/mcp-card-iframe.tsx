"use client";

// Renders an Everruns MCP Apps card (knowledge/ui/mcp-cards.md) inside a
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
  /** Maximum rendered width. Entity-card previews default to 400px. */
  maxWidth?: number | string;
  className?: string;
  /** Optional title for the iframe (a11y). */
  title?: string;
}

/** Maximum messages per second per iframe before drops kick in. */
const MESSAGE_RATE_LIMIT = 10;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

// Per-type structural validation. Hosts that consume `onAction` rely on
// the typed `CardMessage` shape — so a guard that merely checks
// `{ type, payload }` exist would let malformed messages slip through
// while TypeScript treats them as well-formed. Each branch validates
// the required fields named by the corresponding `CardMessage` variant.
function isCardMessage(value: unknown): value is CardMessage {
  if (!isPlainObject(value)) return false;
  if (typeof value.type !== "string") return false;
  if (!isPlainObject(value.payload)) return false;
  const payload = value.payload;

  switch (value.type) {
    case "tool":
      // params is optional but, when present, must be a plain object.
      return (
        isNonEmptyString(payload.toolName) &&
        (payload.params === undefined || isPlainObject(payload.params))
      );
    case "prompt":
      return isNonEmptyString(payload.prompt);
    case "intent":
      return (
        isNonEmptyString(payload.intent) &&
        (payload.params === undefined || isPlainObject(payload.params))
      );
    case "link":
      return isNonEmptyString(payload.url);
    case "notify":
      return isNonEmptyString(payload.message);
    default:
      return false;
  }
}

export function McpCardIframe({
  uri,
  html,
  onAction,
  height = 280,
  maxWidth = 400,
  className,
  title,
}: McpCardIframeProps) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const rateTokensRef = useRef({ tokens: MESSAGE_RATE_LIMIT, lastRefill: Date.now() });

  // Hash the URI for stable iframe identity (helps React reconcile when
  // many cards live on one page).
  const iframeName = useMemo(() => `mcp-card-${hashCode(uri)}`, [uri]);
  const iframeTitle = title ?? `MCP card (${uri})`;

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
      title={iframeTitle}
      data-mcp-card-uri={uri}
      // sandbox lacks allow-same-origin on purpose — keeps the iframe in
      // an opaque origin so it cannot read parent state, cookies, or
      // localStorage. Action dispatch only goes through postMessage.
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      srcDoc={html}
      style={{
        width: "100%",
        maxWidth,
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
