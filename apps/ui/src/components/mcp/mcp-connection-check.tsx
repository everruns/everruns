"use client";

// Connection check line for an OAuth MCP catalog preset. The server runs
// discovery and client registration when the preset is saved and on
// "Check again" (knowledge/integrations/mcp-servers.md), so a host the network
// policy blocks shows here when it is added, not halfway through a login.

import { CheckCircle2, AlertTriangle, CircleDashed, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { McpConnectionCheck } from "@/lib/api/types";

export type ConnectionCheckTone = "ready" | "problem" | "unknown";

/** Plain-language summary of a connection check. */
export function describeConnectionCheck(check: McpConnectionCheck | undefined): {
  tone: ConnectionCheckTone;
  label: string;
} {
  switch (check?.status) {
    case "ready":
      return { tone: "ready", label: "Ready to connect" };
    case "blocked_by_network_policy":
      return {
        tone: "problem",
        label: check.host
          ? `Can't reach ${check.host}: this host is not on the allowed network list`
          : "Can't reach the sign-in service: its host is not on the allowed network list",
      };
    case "unreachable":
      return {
        tone: "problem",
        label: "Couldn't reach the server's sign-in service",
      };
    case "failed":
      return {
        tone: "problem",
        label: check.reason
          ? `Sign-in setup needs attention: ${check.reason}`
          : "Sign-in setup needs attention",
      };
    default:
      return { tone: "unknown", label: "Not checked yet" };
  }
}

const toneClass: Record<ConnectionCheckTone, string> = {
  ready: "text-success",
  problem: "text-warning",
  unknown: "text-muted-foreground",
};

export function McpConnectionCheckLine({
  check,
  canCheck,
  checking,
  onCheck,
}: {
  check: McpConnectionCheck | undefined;
  canCheck: boolean;
  checking: boolean;
  onCheck: () => void;
}) {
  const { tone, label } = describeConnectionCheck(check);
  const Icon = tone === "ready" ? CheckCircle2 : tone === "problem" ? AlertTriangle : CircleDashed;
  const checkedAt = check?.checked_at ? new Date(check.checked_at) : null;
  return (
    <div className="flex max-w-[34ch] flex-col items-start gap-1">
      <span
        className={`flex items-start gap-1 text-xs ${toneClass[tone]}`}
        title={checkedAt ? `Checked ${checkedAt.toLocaleString()}` : undefined}
      >
        <Icon className="mt-px size-3.5 shrink-0" aria-hidden />
        <span>{checking ? "Checking…" : label}</span>
      </span>
      {canCheck && (
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-1.5 text-xs"
          disabled={checking}
          onClick={(event) => {
            event.stopPropagation();
            onCheck();
          }}
          // The row opens the edit dialog on Enter/Space; keep that for the row.
          onKeyDown={(event) => event.stopPropagation()}
        >
          <RefreshCw className="mr-1 size-3" aria-hidden />
          Check again
        </Button>
      )}
    </div>
  );
}
