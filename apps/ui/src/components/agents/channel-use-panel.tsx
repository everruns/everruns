"use client";

import { useMemo } from "react";
import { CodeBlock, type CodeBlockSample } from "@/components/ui/code-block";
import type { AppChannel, ChannelType } from "@/lib/api/types";

/// The channel-scoped ingress path for each transport (EVE-1000). These are
/// addressed by the channel's own public id, which is why the snippet here can
/// show a URL that actually works rather than the placeholder the old Integrate
/// tab printed.
///
/// `null` means the transport has no inbound URL a caller dials: a schedule
/// fires on its own, and Slack is reached through the Slack app rather than by
/// the operator.
function ingressPath(kind: ChannelType, channelId: string): string | null {
  switch (kind) {
    case "webhook":
      return `/v1/e/${channelId}/webhook`;
    case "api_endpoint":
      return `/v1/e/${channelId}/sessions`;
    case "a2a":
      return `/v1/e/${channelId}/a2a`;
    case "ag_ui":
      return `/v1/e/${channelId}/ag-ui`;
    case "fcp":
      return `/v1/e/${channelId}/fcp`;
    case "slack":
      return `/v1/e/${channelId}/slack/events`;
    default:
      return null;
  }
}

function describe(kind: ChannelType): string {
  switch (kind) {
    case "slack":
      return "Point your Slack app's Event Subscriptions request URL here.";
    case "fcp":
      return "Open this URL in a browser, or embed it — it serves the chat surface itself.";
    case "ag_ui":
      return "AG-UI clients POST a run to this URL and read the SSE stream back.";
    case "a2a":
      return "A2A callers send a task here; the agent card is at the same path plus /.well-known/agent-card.json.";
    case "api_endpoint":
      return "Create a session with the channel's API key, then post messages to it.";
    default:
      return "Send a request here to invoke the agent through this channel.";
  }
}

/// "How do I call this channel", rendered inside the expanded channel row.
///
/// This replaces the agent-level Integrate tab (EVE-1009). The tab could only
/// show generic snippets, because at the agent level there is no single URL —
/// an agent reachable through Slack and a webhook has two. Here there is
/// exactly one, so the snippet is the real thing.
export function ChannelUsePanel({ channel }: { channel: AppChannel }) {
  const path = ingressPath(channel.channel_type, channel.id);

  const samples = useMemo<CodeBlockSample[]>(() => {
    if (!path) return [];
    // Resolved at render so the snippet names this deployment's host rather
    // than whatever the build happened to know.
    const origin = typeof window !== "undefined" ? window.location.origin : "";
    const url = `${origin}/api${path}`;

    if (channel.channel_type === "fcp" || channel.channel_type === "ag_ui") {
      return [{ label: "URL", language: "text", code: url }];
    }
    if (channel.channel_type === "slack") {
      return [{ label: "Request URL", language: "text", code: url }];
    }
    return [
      {
        label: "curl",
        language: "bash",
        code: `curl -X POST '${url}' \\\n  -H 'content-type: application/json' \\\n  -d '{"message": "hello"}'`,
      },
      { label: "URL", language: "text", code: url },
    ];
  }, [channel.channel_type, path]);

  if (!path) {
    return (
      <div>
        <p className="text-xs font-medium uppercase text-muted-foreground">Use it</p>
        <p className="mt-1 text-sm text-muted-foreground">
          This channel fires on its own — there is no inbound URL to call.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <div>
        <p className="text-xs font-medium uppercase text-muted-foreground">Use it</p>
        <p className="mt-1 text-sm text-muted-foreground">{describe(channel.channel_type)}</p>
      </div>
      <CodeBlock samples={samples} />
    </div>
  );
}
