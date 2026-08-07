"use client";

import { useEffect } from "react";
import { registerWebMcpTool } from "@/lib/webmcp/register-tool";
import type { WebMcpToolDefinition } from "@/lib/webmcp/types";
import { useWebMcp } from "@/providers/webmcp-context";

export function useWebMcpTool(
  tool: WebMcpToolDefinition,
  options: { enabled?: boolean; scopeKey?: string } = {},
) {
  const webmcp = useWebMcp();
  const enabled = webmcp.enabled && (options.enabled ?? true);

  useEffect(() => {
    if (!enabled) return;

    const controller = new AbortController();
    void registerWebMcpTool(tool, controller.signal).catch((error: unknown) => {
      if (controller.signal.aborted) return;
      const name = error instanceof DOMException ? error.name : "Error";
      console.warn(`WebMCP tool registration failed (${tool.name}, ${name})`);
    });

    return () => controller.abort();
  }, [enabled, options.scopeKey, tool]);
}
