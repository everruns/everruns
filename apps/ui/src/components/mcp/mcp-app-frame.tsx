"use client";

// Runs an MCP App view (SEP-1865) that a connected MCP server ships, as the
// host side of the protocol: a sandboxed frame plus the JSON-RPC bridge over
// postMessage.
//
// Decisions:
// - The frame is `srcdoc` with `sandbox="allow-scripts"` and no
//   `allow-same-origin`, so the view runs in an opaque origin: it cannot read
//   the console's cookies, storage or DOM. A CSP meta tag, built only from the
//   origins the view declared, limits what it can load and reach.
// - Messages are accepted only from this frame's window, and at most
//   MESSAGE_RATE_LIMIT per second.
// - The bridge never acts on its own authority: tool calls, resource reads,
//   links and messages go to callbacks the caller owns, which apply the
//   console's confirmation and permission rules.
// - `ui/update-model-context` and sampling are not offered; the view cannot
//   steer the model.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { buildViewCsp, withCsp, type McpAppViewPolicy } from "@/lib/mcp-apps";

export const MCP_APPS_PROTOCOL_VERSION = "2026-01-26";
const MESSAGE_RATE_LIMIT = 30;
const MIN_HEIGHT = 120;
const MAX_INLINE_HEIGHT = 720;

type JsonRpcId = string | number;
interface JsonRpcMessage {
  jsonrpc: "2.0";
  id?: JsonRpcId;
  method?: string;
  params?: Record<string, unknown>;
  result?: unknown;
  error?: { code: number; message: string };
}

export type McpAppDisplayMode = "inline" | "fullscreen";

export interface McpAppHostHandlers {
  /** `tools/call` from the view. Resolve with a `CallToolResult`. */
  callTool?: (name: string, args: Record<string, unknown>) => Promise<Record<string, unknown>>;
  /** `resources/read` from the view. Resolve with a `ReadResourceResult`. */
  readResource?: (uri: string) => Promise<Record<string, unknown>>;
  /** `ui/open-link`. */
  openLink: (url: string) => void;
  /** `ui/message`: text the view proposes; the console makes it a draft. */
  message: (text: string) => void;
  /** `ui/request-display-mode`. Returns the mode now in effect. */
  requestDisplayMode: (mode: McpAppDisplayMode) => McpAppDisplayMode;
  /** `ui/notifications/request-teardown`. */
  requestTeardown?: () => void;
}

export interface McpAppFrameProps {
  html: string;
  policy: McpAppViewPolicy;
  /** Tool definition for `hostContext.toolInfo`. */
  tool: { name: string; title?: string; description?: string };
  toolCallId: string;
  toolInput: Record<string, unknown>;
  /** The call's `CallToolResult`; sent once the view is initialized. */
  toolResult: Record<string, unknown>;
  displayMode: McpAppDisplayMode;
  theme: "light" | "dark";
  locale: string;
  handlers: McpAppHostHandlers;
  title: string;
  className?: string;
  /** Fixed height (fullscreen); otherwise the view's reported size is used. */
  height?: number | string;
  onReady?: () => void;
  onHeight?: (height: number) => void;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function McpAppFrame({
  html,
  policy,
  tool,
  toolCallId,
  toolInput,
  toolResult,
  displayMode,
  theme,
  locale,
  handlers,
  title,
  className,
  height,
  onReady,
  onHeight,
}: McpAppFrameProps) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const initializedRef = useRef(false);
  const bucketRef = useRef({ tokens: MESSAGE_RATE_LIMIT, last: 0 });
  const nextIdRef = useRef(1);
  // Latest values for the message handler without re-subscribing.
  const live = useRef({ handlers, toolInput, toolResult, displayMode, theme, locale, tool });
  useLayoutEffect(() => {
    live.current = { handlers, toolInput, toolResult, displayMode, theme, locale, tool };
  });

  const srcDoc = useMemo(() => withCsp(html, buildViewCsp(policy)), [html, policy]);

  const post = useCallback((message: JsonRpcMessage) => {
    iframeRef.current?.contentWindow?.postMessage(message, "*");
  }, []);

  const hostContext = useCallback(() => {
    const { displayMode, theme, locale, tool } = live.current;
    const width = iframeRef.current?.clientWidth ?? 0;
    return {
      toolInfo: {
        id: toolCallId,
        tool: {
          name: tool.name,
          title: tool.title,
          description: tool.description,
          inputSchema: { type: "object" },
        },
      },
      theme,
      displayMode,
      availableDisplayModes: ["inline", "fullscreen"],
      containerDimensions:
        displayMode === "fullscreen"
          ? { width, height: iframeRef.current?.clientHeight ?? 0 }
          : { width, maxHeight: MAX_INLINE_HEIGHT },
      locale,
      timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      platform: "web",
    };
  }, [toolCallId]);

  // Tell an initialized view when the display mode or theme changes.
  useEffect(() => {
    if (!initializedRef.current) return;
    post({
      jsonrpc: "2.0",
      method: "ui/notifications/host-context-changed",
      params: hostContext(),
    });
  }, [displayMode, theme, post, hostContext]);

  // Give the view a chance to clean up before it goes away.
  useEffect(() => {
    return () => {
      if (initializedRef.current) {
        post({
          jsonrpc: "2.0",
          id: `teardown-${nextIdRef.current++}`,
          method: "ui/resource-teardown",
          params: {},
        });
      }
    };
  }, [post]);

  useEffect(() => {
    function reply(id: JsonRpcId, result: unknown) {
      post({ jsonrpc: "2.0", id, result });
    }
    function fail(id: JsonRpcId, message: string, code = -32603) {
      post({ jsonrpc: "2.0", id, error: { code, message } });
    }

    async function handleRequest(id: JsonRpcId, method: string, params: Record<string, unknown>) {
      const { handlers } = live.current;
      switch (method) {
        case "ui/initialize":
          reply(id, {
            protocolVersion: MCP_APPS_PROTOCOL_VERSION,
            hostInfo: { name: "Everruns", version: "1" },
            hostCapabilities: {
              openLinks: {},
              logging: {},
              message: { text: {} },
              ...(handlers.callTool ? { serverTools: {} } : {}),
              ...(handlers.readResource ? { serverResources: {} } : {}),
            },
            hostContext: hostContext(),
          });
          return;
        case "ping":
          reply(id, {});
          return;
        case "tools/call": {
          if (!handlers.callTool) return fail(id, "This view cannot call tools here", -32601);
          const name = params.name;
          if (typeof name !== "string" || !name) return fail(id, "Missing tool name", -32602);
          const args = isPlainObject(params.arguments) ? params.arguments : {};
          try {
            reply(id, await handlers.callTool(name, args));
          } catch (error) {
            fail(id, error instanceof Error ? error.message : "Tool call failed");
          }
          return;
        }
        case "resources/read": {
          if (!handlers.readResource)
            return fail(id, "This view cannot read resources here", -32601);
          const uri = params.uri;
          if (typeof uri !== "string" || !uri) return fail(id, "Missing resource URI", -32602);
          try {
            reply(id, await handlers.readResource(uri));
          } catch (error) {
            fail(id, error instanceof Error ? error.message : "Resource read failed");
          }
          return;
        }
        case "ui/open-link": {
          const url = params.url;
          if (typeof url !== "string" || !/^https?:\/\//i.test(url)) {
            return reply(id, { isError: true });
          }
          handlers.openLink(url);
          return reply(id, {});
        }
        case "ui/message": {
          const content = Array.isArray(params.content) ? params.content : [];
          const text = content
            .filter(
              (block): block is { type: string; text: string } =>
                isPlainObject(block) && block.type === "text" && typeof block.text === "string",
            )
            .map((block) => block.text)
            .join("\n")
            .trim();
          if (!text) return reply(id, { isError: true });
          handlers.message(text);
          return reply(id, {});
        }
        case "ui/request-display-mode": {
          const mode = params.mode === "fullscreen" ? "fullscreen" : "inline";
          return reply(id, { mode: handlers.requestDisplayMode(mode) });
        }
        default:
          // ui/update-model-context, ui/download-file, sampling: not offered.
          return fail(id, `Method not supported: ${method}`, -32601);
      }
    }

    function handleNotification(method: string, params: Record<string, unknown>) {
      switch (method) {
        case "ui/notifications/initialized": {
          initializedRef.current = true;
          const { toolInput, toolResult } = live.current;
          post({
            jsonrpc: "2.0",
            method: "ui/notifications/tool-input",
            params: { arguments: toolInput },
          });
          post({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: toolResult });
          onReady?.();
          return;
        }
        case "ui/notifications/size-changed": {
          const reported = typeof params.height === "number" ? params.height : null;
          if (reported !== null && Number.isFinite(reported)) {
            onHeight?.(Math.min(MAX_INLINE_HEIGHT, Math.max(MIN_HEIGHT, Math.ceil(reported))));
          }
          return;
        }
        case "ui/notifications/request-teardown":
          live.current.handlers.requestTeardown?.();
          return;
        default:
          // notifications/message (logging) and anything else: ignored.
          return;
      }
    }

    function onMessage(event: MessageEvent) {
      const frame = iframeRef.current;
      if (!frame || event.source !== frame.contentWindow) return;
      const data = event.data as unknown;
      if (!isPlainObject(data) || data.jsonrpc !== "2.0") return;
      const now = Date.now();
      const bucket = bucketRef.current;
      bucket.tokens = Math.min(
        MESSAGE_RATE_LIMIT,
        bucket.tokens + ((now - bucket.last) / 1000) * MESSAGE_RATE_LIMIT,
      );
      bucket.last = now;
      if (bucket.tokens < 1) return;
      bucket.tokens -= 1;

      const method = typeof data.method === "string" ? data.method : null;
      const params = isPlainObject(data.params) ? data.params : {};
      const id = typeof data.id === "string" || typeof data.id === "number" ? data.id : null;
      if (!method) return; // responses to our own requests carry nothing we wait on
      if (id !== null) void handleRequest(id, method, params);
      else handleNotification(method, params);
    }

    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [post, hostContext, onReady, onHeight]);

  return (
    <iframe
      ref={iframeRef}
      title={title}
      // No allow-same-origin: the view stays in an opaque origin.
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      srcDoc={srcDoc}
      data-testid="mcp-app-frame"
      style={{
        width: "100%",
        height: height ?? 360,
        border: "none",
        background: "transparent",
        display: "block",
      }}
      className={className}
    />
  );
}
