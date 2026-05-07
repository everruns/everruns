"use client";

import { McpCardIframe } from "@/components/mcp/mcp-card-iframe";
import type { McpAppResource } from "./tool-call-utils";

interface McpAppResourceListProps {
  resources: McpAppResource[];
  className?: string;
}

export function McpAppResourceList({ resources, className }: McpAppResourceListProps) {
  if (resources.length === 0) return null;

  return (
    <div className={className ?? "mt-2 space-y-2"}>
      {resources.map((resource) => (
        <McpCardIframe
          key={resource.uri}
          uri={resource.uri}
          html={resource.html}
          height={360}
          maxWidth="100%"
          title={`MCP App (${resource.uri})`}
          className="rounded-md border border-border/70 bg-background"
        />
      ))}
    </div>
  );
}
