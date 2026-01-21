"use client";

import Link from "next/link";
import { Info, MessageSquare, Loader2, Zap } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { ProviderIcon } from "@/components/providers/provider-icon";
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { formatRelativeTime, formatTokens } from "@/lib/formatting";
import type { Session, SessionStatus, LlmModelWithProvider, TokenUsage } from "@/lib/api/types";

/**
 * Format total tokens from usage
 */
function formatTotalTokens(usage: TokenUsage): string {
  const total = usage.input_tokens + usage.output_tokens;
  return formatTokens(total);
}

export interface SessionCardProps {
  /** The session to display */
  session: Session;
  /** The agent ID (for building links) */
  agentId: string;
  /** Optional LLM model for display */
  model?: LlmModelWithProvider;
  /** Optional custom summary text (overrides default title display) */
  summary?: string;
  /** Whether to show the delete button (if provided with onDelete) */
  onDelete?: (sessionId: string, sessionTitle: string) => void;
}

/**
 * Truncate text to a specified number of lines (approximate by character limit)
 * Each "line" is approximately 60 characters
 */
function truncateToLines(text: string, lines: number = 2): string {
  const maxChars = lines * 60;
  if (text.length <= maxChars) return text;
  return text.slice(0, maxChars).trim() + "...";
}

/**
 * Get display status info for a session status
 */
function getStatusInfo(status: SessionStatus): {
  label: string;
  variant: "default" | "secondary" | "outline";
  isRunning: boolean;
} {
  switch (status) {
    case "active":
      return { label: "Running", variant: "default", isRunning: true };
    case "idle":
      return { label: "Idle", variant: "secondary", isRunning: false };
    case "started":
    default:
      return { label: "New", variant: "outline", isRunning: false };
  }
}

/**
 * Session info icon that displays session metadata in a tooltip.
 * Styled to match MessageInfoIcon component.
 */
function SessionInfoIcon({ session }: { session: Session }) {
  const formattedCreatedAt = new Date(session.created_at).toLocaleString();
  const formattedStartedAt = session.started_at
    ? new Date(session.started_at).toLocaleString()
    : null;

  return (
    <Tooltip>
      <TooltipTrigger
        className={cn(
          "p-0.5 rounded transition-colors flex-shrink-0",
          "text-muted-foreground/50 hover:text-muted-foreground hover:bg-muted/80"
        )}
        aria-label="Session info"
        onClick={(e) => e.preventDefault()} // Prevent link navigation when clicking info
      >
        <Info className="w-3 h-3" />
      </TooltipTrigger>
      <TooltipContent className="max-w-sm">
        <div className="space-y-1 text-xs">
          <div className="flex gap-2">
            <span className="text-muted-foreground">ID:</span>
            <span className="font-mono text-[10px] break-all">{session.id}</span>
          </div>
          <div className="flex gap-2">
            <span className="text-muted-foreground">Status:</span>
            <span className="capitalize">{session.status}</span>
          </div>
          <div className="flex gap-2">
            <span className="text-muted-foreground">Created:</span>
            <span>{formattedCreatedAt}</span>
          </div>
          {formattedStartedAt && (
            <div className="flex gap-2">
              <span className="text-muted-foreground">Started:</span>
              <span>{formattedStartedAt}</span>
            </div>
          )}
          {session.tags.length > 0 && (
            <div className="flex gap-2">
              <span className="text-muted-foreground">Tags:</span>
              <span>{session.tags.join(", ")}</span>
            </div>
          )}
        </div>
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * SessionCard displays a session with status, summary, and metadata.
 * Designed for use in session lists and overview pages.
 */
export function SessionCard({
  session,
  agentId,
  model,
  summary,
}: SessionCardProps) {
  const statusInfo = getStatusInfo(session.status);
  const displayTitle = session.title || `Session ${session.id.slice(0, 8)}`;
  // Show preview from session (first user message), explicit summary prop, or nothing
  const inputPreview = summary ?? session.preview;
  // Show output preview from session (last assistant message)
  const outputPreview = session.output_preview;

  return (
    <Link
      href={`/agents/${agentId}/sessions/${session.id}`}
      className="flex items-start justify-between p-3 rounded-md border hover:bg-muted transition-colors group"
    >
      <div className="flex items-start gap-3 flex-1 min-w-0">
        <div className="flex-shrink-0 mt-0.5">
          {statusInfo.isRunning ? (
            <Loader2 className="w-4 h-4 text-primary animate-spin" />
          ) : (
            <MessageSquare className="w-4 h-4 text-muted-foreground" />
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <p className="font-medium truncate">{displayTitle}</p>
            <Badge variant={statusInfo.variant} className="flex-shrink-0 text-xs">
              {statusInfo.label}
            </Badge>
          </div>
          {/* Input preview: first user message */}
          {inputPreview && (
            <p className="text-sm text-muted-foreground mt-1 line-clamp-1">
              {truncateToLines(inputPreview, 1)}
            </p>
          )}
          {/* Output preview: last assistant response */}
          {outputPreview && (
            <p className="text-sm text-muted-foreground/70 mt-0.5 line-clamp-1 italic">
              {truncateToLines(outputPreview, 1)}
            </p>
          )}
          <div className="flex items-center gap-3 mt-1 text-xs text-muted-foreground">
            <span>{formatRelativeTime(session.created_at)}</span>
            {session.usage && (session.usage.input_tokens > 0 || session.usage.output_tokens > 0) && (
              <span className="flex items-center gap-1" title="Token usage">
                <Zap className="w-3 h-3" />
                {formatTotalTokens(session.usage)}
              </span>
            )}
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2 flex-shrink-0 ml-2">
        {model && (
          <Badge variant="outline" className="gap-1 text-xs">
            <ProviderIcon
              providerType={model.provider_type}
              size="sm"
              showBackground={false}
            />
            {model.display_name}
          </Badge>
        )}
        <SessionInfoIcon session={session} />
      </div>
    </Link>
  );
}

/**
 * Export SessionInfoIcon for use in other components if needed
 */
export { SessionInfoIcon };
