"use client";

import {
  Info,
  MessageSquare,
  Loader2,
  Zap,
  Pin,
  PinOff,
  CalendarClock,
  Download,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { EntityCard } from "@/components/ui/entity-card";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuPositioner,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import { cn, shortenId } from "@/lib/utils";
import { formatRelativeTime, formatTokens } from "@/lib/formatting";
import { useLocale } from "@/providers/locale-provider";
import type { Session, SessionStatus, ModelWithProvider, TokenUsage } from "@/lib/api/types";
import type { SessionExportFormat } from "@/lib/api/sessions";
import { getEntityReferenceClassName, getEntityReferenceLabel } from "@/lib/entity-lifecycle";
import { joinTags } from "@/lib/tags";

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
  /** Optional agent name to display (for org-level session lists) */
  agentName?: string;
  /** Optional agent lifecycle status for reference rendering */
  agentStatus?: string | null;
  /** Optional LLM model for display */
  model?: ModelWithProvider;
  /** Optional custom summary text (overrides default title display) */
  summary?: string;
  /** Whether to show the delete button (if provided with onDelete) */
  onDelete?: (sessionId: string, sessionTitle: string) => void;
  /** Callback to toggle pin state */
  onTogglePin?: (sessionId: string, pinned: boolean) => void;
  /** Callback to export the session in the chosen format */
  onExport?: (sessionId: string, format: SessionExportFormat) => void;
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
  const tagList = joinTags(session.tags);

  return (
    <Tooltip>
      <TooltipTrigger
        className={cn(
          "p-0.5 rounded transition-colors flex-shrink-0",
          "text-muted-foreground/50 hover:text-muted-foreground hover:bg-muted/80",
        )}
        aria-label="Session info"
        onClick={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
      >
        <Info className="w-3 h-3" />
      </TooltipTrigger>
      <TooltipContent className="max-w-sm">
        <div className="space-y-1 text-xs">
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
          {tagList && (
            <div className="flex gap-2">
              <span className="text-muted-foreground">Tags:</span>
              <span>{tagList}</span>
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
  agentName,
  agentStatus,
  model,
  summary,
  onTogglePin,
  onExport,
}: SessionCardProps) {
  const { t } = useLocale();
  const statusInfo = getStatusInfo(session.status);
  const displayTitle = session.title || `Session ${shortenId(session.id)}`;
  // Show preview from session (first user message), explicit summary prop, or nothing
  const inputPreview = summary ?? session.preview;
  // Show output preview from session (last assistant message)
  const outputPreview = session.output_preview;
  // Sessions are org-level entities
  const sessionUrl = `/sessions/${session.id}/transcript`;
  const isPinned = session.is_pinned === true;

  return (
    <EntityCard
      variant="row"
      href={sessionUrl}
      title={displayTitle}
      copyValue={session.id}
      icon={
        statusInfo.isRunning ? (
          <Loader2 className="w-4 h-4 text-primary animate-spin" />
        ) : (
          <MessageSquare className="w-4 h-4 text-muted-foreground" />
        )
      }
      inlineBadges={
        <>
          <Badge variant={statusInfo.variant} className="flex-shrink-0 text-xs">
            {statusInfo.label}
          </Badge>
          {agentName && (
            <Badge variant="outline" className="flex-shrink-0 text-xs">
              <span className={getEntityReferenceClassName(agentStatus)}>
                {getEntityReferenceLabel({
                  kind: "Agent",
                  name: agentName,
                  status: agentStatus,
                })}
              </span>
            </Badge>
          )}
        </>
      }
      headerActions={
        <>
          {model && (
            <Badge variant="outline" className="gap-1 text-xs">
              <ProviderIcon providerType={model.provider_type} size="sm" showBackground={false} />
              {model.display_name}
            </Badge>
          )}
          {onExport && (
            <DropdownMenu>
              <DropdownMenuTrigger
                className={cn(
                  "p-0.5 rounded transition-colors flex-shrink-0",
                  "text-muted-foreground/50 hover:text-muted-foreground hover:bg-muted/80",
                )}
                aria-label={t("export_session")}
              >
                <Download className="w-3.5 h-3.5" />
              </DropdownMenuTrigger>
              <DropdownMenuPositioner align="end">
                <DropdownMenuContent>
                  <DropdownMenuItem onClick={() => onExport(session.id, "jsonl")}>
                    {t("export_jsonl")}
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => onExport(session.id, "atif")}>
                    {t("export_atif")}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenuPositioner>
            </DropdownMenu>
          )}
          {onTogglePin && (
            <Tooltip>
              <TooltipTrigger
                className={cn(
                  "p-0.5 rounded transition-colors flex-shrink-0",
                  isPinned
                    ? "text-primary hover:text-primary/80 hover:bg-muted/80"
                    : "text-muted-foreground/50 hover:text-muted-foreground hover:bg-muted/80",
                )}
                aria-label={isPinned ? "Unpin session" : "Pin session"}
                onClick={() => onTogglePin(session.id, !isPinned)}
              >
                {isPinned ? <PinOff className="w-3.5 h-3.5" /> : <Pin className="w-3.5 h-3.5" />}
              </TooltipTrigger>
              <TooltipContent>{isPinned ? "Unpin session" : "Pin session"}</TooltipContent>
            </Tooltip>
          )}
          <SessionInfoIcon session={session} />
        </>
      }
    >
      {session.goal && (
        <p className="text-sm text-foreground/80 mt-1 line-clamp-1">
          {truncateToLines(session.goal, 1)}
        </p>
      )}
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
        {(session.active_schedule_count ?? 0) > 0 && (
          <span className="flex items-center gap-1" title="Active schedules">
            <CalendarClock className="w-3 h-3" />
            {session.active_schedule_count}
          </span>
        )}
      </div>
    </EntityCard>
  );
}

/**
 * Export SessionInfoIcon for use in other components if needed
 */
export { SessionInfoIcon };
