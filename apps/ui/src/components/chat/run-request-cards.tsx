"use client";

/**
 * Questions and approvals from the agents this chat handed work to
 * (knowledge/runtime-resources/agents-using-agents.md).
 *
 * A request from any agent in a run is answered where the person talks: the
 * run's first session. Each card is the same card the chat shows for its own
 * agent, headed with the asking agent's name, and answers on the asking
 * agent's session. The session page never asks; only chat surfaces render this.
 *
 * The other agents' sessions have their own event streams, so the list is
 * read from the first session's `run-requests` and refreshed while the chat is
 * open: often while a turn runs, rarely when idle, because background work can
 * still ask.
 */

import { useUiTranslations } from "@/providers/locale-provider";
import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { getRunRequests } from "@/lib/api/sessions";
import type { RunRequest } from "@/lib/api/schema-types";
import type { ToolCompletedData } from "@/lib/api/types";
import { AgentAvatar } from "@/components/chat/agent-avatar";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
import {
  UrlElicitationToolCall,
  type UrlElicitationArguments,
} from "@/components/chat/url-elicitation-tool-call";
import {
  MCP_APPROVAL_TOOL,
  McpApprovalToolCall,
  type McpApprovalArguments,
} from "@/components/chat/mcp-approval-tool-call";
import {
  TOOL_APPROVAL_TOOL,
  ToolApprovalRequests,
} from "@/components/chat/tool-approval-tool-call";
import { AskUserToolCall, isAskUserArguments } from "@/components/chat/ask-user-tool-call";

const ACTIVE_REFRESH_MS = 3_000;
const IDLE_REFRESH_MS = 15_000;

// A request answered on the asking agent's session closes once the list
// refreshes; there is no earlier result to show here.
const NO_RESULTS = new Map<string, ToolCompletedData>();

interface AgentRequests {
  sessionId: string;
  agentName: string | null;
  requests: RunRequest[];
}

/** Requests grouped by the asking agent's session, in the order they arrived. */
export function groupRunRequests(requests: RunRequest[]): AgentRequests[] {
  const groups = new Map<string, AgentRequests>();
  for (const request of requests) {
    const group = groups.get(request.session_id) ?? {
      sessionId: request.session_id,
      agentName: request.agent?.name ?? null,
      requests: [],
    };
    group.requests.push(request);
    groups.set(request.session_id, group);
  }
  return [...groups.values()];
}

export function RunRequestCards({
  sessionId,
  sessionActive,
}: {
  sessionId: string;
  sessionActive: boolean;
}) {
  const ui = useUiTranslations();
  const { data } = useQuery({
    queryKey: ["sessions", sessionId, "run-requests"],
    queryFn: () => getRunRequests(sessionId),
    refetchInterval: sessionActive ? ACTIVE_REFRESH_MS : IDLE_REFRESH_MS,
  });
  const groups = useMemo(() => groupRunRequests(data?.requests ?? []), [data]);
  if (groups.length === 0) return null;

  return (
    <div className="mt-4 space-y-4" data-slot="run-requests">
      {groups.map((group) => {
        const name = group.agentName ?? ui("Another agent");
        const approvals = group.requests.filter(
          (request) => request.tool_call.name === TOOL_APPROVAL_TOOL,
        );
        return (
          <section
            key={group.sessionId}
            aria-label={ui("{agent} needs you", { agent: name })}
            className="space-y-2"
          >
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <AgentAvatar name={group.agentName} size="sm" />
              <span>
                <span className="font-medium text-foreground">{name}</span>{" "}
                {ui("is waiting for you")}
              </span>
            </div>
            {approvals.length > 0 && (
              <ToolApprovalRequests
                sessionId={group.sessionId}
                requests={approvals.map((request) => ({
                  id: request.tool_call.id,
                  arguments: request.tool_call.arguments,
                }))}
                toolResultsMap={NO_RESULTS}
              />
            )}
            {group.requests.map((request) => (
              <RunRequestCard
                key={request.tool_call.id}
                sessionId={group.sessionId}
                request={request}
              />
            ))}
          </section>
        );
      })}
    </div>
  );
}

/** One card other than an approval, which renders with its batch above. */
function RunRequestCard({ sessionId, request }: { sessionId: string; request: RunRequest }) {
  const { id, name, arguments: args } = request.tool_call;
  switch (name) {
    case "ask_user":
      return isAskUserArguments(args) ? (
        <AskUserToolCall
          sessionId={sessionId}
          toolCallId={id}
          request={args}
          requestedAt={request.requested_at}
          toolResultsMap={NO_RESULTS}
        />
      ) : null;
    case MCP_APPROVAL_TOOL:
      return (
        <McpApprovalToolCall
          sessionId={sessionId}
          toolCallId={id}
          approval={(args ?? {}) as McpApprovalArguments}
          toolResultsMap={NO_RESULTS}
        />
      );
    case "confirm_url_elicitation":
      return (
        <UrlElicitationToolCall
          sessionId={sessionId}
          toolCallId={id}
          elicitation={(args ?? {}) as UrlElicitationArguments}
          toolResultsMap={NO_RESULTS}
        />
      );
    case "setup_connection": {
      const setup = (args ?? {}) as {
        provider?: string;
        subject?: "agent" | "user";
        setup_url?: string;
      };
      return (
        <SetupConnectionToolCall
          sessionId={sessionId}
          toolCallId={id}
          provider={setup.provider ?? "unknown"}
          subject={setup.subject}
          setupUrl={setup.setup_url}
          toolResultsMap={NO_RESULTS}
        />
      );
    }
    default:
      return null;
  }
}
