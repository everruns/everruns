"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { MessageSquare, Loader2, Zap, Sparkles, Bot } from "lucide-react";
import { formatRelativeTime, formatTokens } from "@/lib/formatting";
import { shortenId } from "@/lib/utils";
import type { Session, Agent, LlmModelWithProvider, TokenUsage } from "@/lib/api/types";
import { getEntityReferenceClassName, getEntityReferenceLabel } from "@/lib/entity-lifecycle";

interface RecentSessionsProps {
  sessions: Session[];
  agents: Agent[];
  models?: LlmModelWithProvider[];
}

function truncateText(text: string, maxLength: number = 80): string {
  if (text.length <= maxLength) return text;
  return text.slice(0, maxLength).trim() + "...";
}

function formatTotalTokens(usage: TokenUsage): string {
  const total = usage.input_tokens + usage.output_tokens;
  return formatTokens(total);
}

export function RecentSessions({ sessions, agents, models = [] }: RecentSessionsProps) {
  const recentSessions = sessions.slice(0, 10);
  const agentMap = new Map(agents.map((a) => [a.id, a]));
  const modelMap = new Map(models.map((m) => [m.id, m]));

  const getStatusBadge = (session: Session) => {
    switch (session.status) {
      case "active":
        return (
          <Badge variant="default" className="text-xs">
            Running
          </Badge>
        );
      case "idle":
        return (
          <Badge variant="secondary" className="text-xs">
            Idle
          </Badge>
        );
      case "started":
      default:
        return (
          <Badge variant="outline" className="text-xs">
            New
          </Badge>
        );
    }
  };

  const isRunning = (session: Session) => session.status === "active";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent Sessions</CardTitle>
      </CardHeader>
      <CardContent>
        {recentSessions.length === 0 ? (
          <p className="text-center text-muted-foreground py-8">
            No sessions yet. Create an agent and start a session to begin.
          </p>
        ) : (
          <div className="space-y-2">
            {recentSessions.map((session) => {
              const agent = session.agent_id ? agentMap.get(session.agent_id) : undefined;
              const model = session.model_id ? modelMap.get(session.model_id) : undefined;
              const agentLabel = session.agent_id
                ? getEntityReferenceLabel({
                    kind: "Agent",
                    name: agent?.name,
                    status: agent?.status ?? "deleted",
                  })
                : null;
              return (
                <Link
                  key={session.id}
                  href={`/sessions/${session.id}`}
                  className="flex items-start gap-3 border bg-card p-3 transition-colors hover:bg-muted/50"
                >
                  {/* Icon */}
                  <div className="flex-shrink-0 mt-0.5">
                    {isRunning(session) ? (
                      <Loader2 className="w-4 h-4 text-primary animate-spin" />
                    ) : (
                      <MessageSquare className="icon-sharp h-4 w-4 text-muted-foreground" />
                    )}
                  </div>

                  {/* Content */}
                  <div className="flex-1 min-w-0">
                    {/* Title row */}
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-medium truncate">
                        {session.title || `Session ${shortenId(session.id)}`}
                      </span>
                      {getStatusBadge(session)}
                      {agentLabel && (
                        <span className="text-xs text-muted-foreground flex items-center gap-1">
                          <Bot className="icon-sharp h-3 w-3" />
                          <span className={getEntityReferenceClassName(agent?.status ?? "deleted")}>
                            {agentLabel}
                          </span>
                        </span>
                      )}
                    </div>

                    {/* Input preview */}
                    {session.preview && (
                      <p className="text-sm text-muted-foreground mt-1 truncate">
                        {truncateText(session.preview)}
                      </p>
                    )}

                    {/* Output preview */}
                    {session.output_preview && (
                      <p className="text-sm text-muted-foreground/70 mt-0.5 truncate italic">
                        {truncateText(session.output_preview)}
                      </p>
                    )}

                    {/* Meta row */}
                    <div className="flex items-center gap-3 mt-1 text-xs text-muted-foreground">
                      <span>{formatRelativeTime(session.created_at)}</span>
                      {model && (
                        <span className="flex items-center gap-1">
                          <Sparkles className="icon-sharp h-3 w-3" />
                          {model.display_name}
                        </span>
                      )}
                      {session.usage &&
                        (session.usage.input_tokens > 0 || session.usage.output_tokens > 0) && (
                          <span className="flex items-center gap-1">
                            <Zap className="icon-sharp h-3 w-3" />
                            {formatTotalTokens(session.usage)}
                          </span>
                        )}
                    </div>
                  </div>
                </Link>
              );
            })}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
