"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import type { McpClientAttribution } from "@/lib/mcp-attribution";

/** Names come from registration, not a verified publisher directory. */
export function McpClientDetails({ client }: { client: McpClientAttribution }) {
  const ui = useUiTranslations();
  return (
    <div className="min-w-0 space-y-1 text-xs">
      <div>{ui("Via MCP: {name} (self-declared)", { name: client.client_name })}</div>
      <div className="break-all text-muted-foreground">
        {ui("Client ID")}: {client.client_id}
      </div>
      <div className="break-all text-muted-foreground">
        {ui("Grant ID")}: {client.grant_id}
      </div>
    </div>
  );
}
