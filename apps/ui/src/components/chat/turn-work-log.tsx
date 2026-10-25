/**
 * Decisions:
 * - The work log starts collapsed and stays collapsed while the turn runs. The
 *   header carries the live signal instead: a shimmering "Working for 12s", the
 *   latest step as a one-line status, and a count of failed tool calls. Tool
 *   rows only show when the user opens the section, and their choice sticks
 *   across the active → completed transition (no auto-open, no auto-collapse).
 * - Failures stay discoverable while collapsed through the header's counts. A
 *   failed tool call went back to the model and the turn carried on, so it is
 *   an "issue": amber warning triangle, never destructive red. Red is kept for
 *   runtime-marked errors, i.e. work that stopped (EVE-1236).
 * - `pinned` (the work report's plan while running, its result once done) and
 *   `attention` render outside the collapsible body, so the plan and cards that need the
 *   user (approvals, ask_user, connection setup) are never hidden by the fold.
 * - The body mounts only while open (and through the collapse transition). A
 *   turn can carry thousands of tool calls; rendering them folded made every
 *   streamed event re-render the whole hidden log. Pass `children` as a
 *   function so the folded log does not even build the elements.
 * - With `phase`, the same element is the turn status row from Enter on
 *   (knowledge/ui/chat-experience.md): Sending, Still connecting, Queued, then
 *   the work log. Only label, status, lead glyph and sheen change, so it never
 *   remounts. The chevron is dim and inert until the fold has steps.
 */
"use client";

import { useEffect, useId, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatWorkLogErrorCount, formatWorkLogIssueCount } from "@/lib/i18n";
import { FailureIcon, failureTextClass } from "@/components/chat/failure-severity";
import { formatWorkedDuration } from "@/components/chat/turn-delimiter";
import { useLocale } from "@/providers/locale-provider";
import { STILL_STARTING_MS, isLiveTurnPhase, type TurnRowPhase } from "@/lib/chat-turn-state";

interface TurnWorkLogProps {
  /** Turn status row phase; when set it drives the label instead of `label`/`isActive`. */
  phase?: TurnRowPhase;
  /** Final duration for done and stopped phases. */
  durationMs?: number;
  /** Whether the fold has steps; without them the chevron is dim and inert. */
  hasSteps?: boolean;
  /** Label for completed work, e.g. "Worked for 33s". Ignored while active. */
  label?: string;
  isActive?: boolean;
  /** Turn start (epoch ms); drives the live "Working for …" counter. */
  startedAtMs?: number;
  /** Latest step, shown next to the label while the turn runs. */
  status?: string;
  /** Recoverable failures (failed tool calls the turn carried on from). */
  issueCount?: number;
  /** Failures the runtime marked fatal. */
  errorCount?: number;
  /** Work report content shown under the header: the plan, or the result. */
  pinned?: ReactNode;
  /** Content that must stay visible regardless of the fold. */
  attention?: ReactNode;
  children: ReactNode | (() => ReactNode);
}

function useElapsedMs(startedAtMs: number | undefined, isActive: boolean): number | null {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!isActive || startedAtMs == null) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [isActive, startedAtMs]);

  if (!isActive || startedAtMs == null) return null;
  return Math.max(0, now - startedAtMs);
}

export function TurnWorkLog({
  phase,
  durationMs,
  hasSteps = true,
  label = "",
  isActive: isActiveProp = false,
  startedAtMs,
  status,
  issueCount = 0,
  errorCount = 0,
  pinned,
  attention,
  children,
}: TurnWorkLogProps) {
  const { locale, t } = useLocale();
  const [expanded, setExpanded] = useState(false);
  // True from the first open until the collapse transition finishes.
  const [bodyMounted, setBodyMounted] = useState(false);
  const bodyId = useId();
  const isActive = phase ? isLiveTurnPhase(phase) : isActiveProp;
  const timed = phase ? phase === "starting" || phase === "thinking" || phase === "working" : true;
  const elapsedMs = useElapsedMs(startedAtMs, isActive && timed);
  const canOpen = hasSteps;
  const open = expanded && canOpen;

  let headerLabel: string;
  let statusText = isActive ? status : undefined;
  if (phase === "sending") {
    headerLabel = t("turn_sending");
  } else if (phase === "connecting") {
    headerLabel = t("turn_still_connecting");
  } else if (phase === "queued") {
    headerLabel = t("turn_queued");
    statusText = t("turn_queued_status");
  } else if (phase === "done") {
    headerLabel = t("worked_for", {
      duration: formatWorkedDuration(durationMs ?? 0),
    });
  } else if (phase === "stopped") {
    headerLabel = t("turn_stopped_after", {
      duration: formatWorkedDuration(durationMs ?? 0),
    });
  } else if (phase) {
    const seconds = Math.floor((elapsedMs ?? 0) / 1000);
    headerLabel = t("working_for", {
      duration: `${seconds < 60 ? `${seconds}s` : formatWorkedDuration(seconds * 1000)}`,
    });
    if (phase === "starting") {
      statusText =
        (elapsedMs ?? 0) >= STILL_STARTING_MS ? t("turn_waiting_for_worker") : t("turn_starting");
    } else if (phase === "thinking") statusText = status || t("turn_thinking");
  } else {
    headerLabel = isActive
      ? elapsedMs != null && elapsedMs >= 1000
        ? t("working_for", {
            duration: formatWorkedDuration(Math.floor(elapsedMs / 1000) * 1000),
          })
        : t("working")
      : label;
  }
  const showStatus = isActive && !!statusText;

  return (
    <div className="animate-chat-row-in space-y-2" data-turn-phase={phase}>
      <button
        type="button"
        aria-expanded={canOpen ? open : undefined}
        aria-controls={canOpen ? bodyId : undefined}
        aria-disabled={!canOpen || undefined}
        onClick={() => {
          if (!canOpen) return;
          if (!expanded) setBodyMounted(true);
          setExpanded(!expanded);
        }}
        className={cn(
          "group flex w-full min-w-0 items-center gap-2 pt-2 text-left text-sm text-muted-foreground transition-colors",
          canOpen ? "hover:text-foreground" : "cursor-default",
        )}
      >
        <span className="flex min-h-5 min-w-5 items-center justify-center">
          {phase === "queued" ? (
            <span
              className="animate-turn-dot-pulse h-1.5 w-1.5 rounded-full bg-muted-foreground"
              data-testid="turn-queued-dot"
            />
          ) : (
            <ChevronRight
              className={cn(
                "h-4 w-4 transition-[transform,opacity] duration-200 ease-out motion-reduce:transition-none",
                open && "rotate-90",
                !canOpen && "opacity-40",
              )}
            />
          )}
        </span>
        <span
          className={cn(
            "whitespace-nowrap font-medium tabular-nums",
            isActive && phase !== "queued" && "work-log-shimmer",
          )}
          aria-live="off"
        >
          {headerLabel}
        </span>
        {errorCount > 0 && (
          <span
            className={cn(
              "inline-flex shrink-0 items-center gap-1 whitespace-nowrap text-xs font-medium",
              failureTextClass("error"),
            )}
            data-testid="work-log-error-count"
          >
            <FailureIcon severity="error" />
            {formatWorkLogErrorCount(locale, errorCount)}
          </span>
        )}
        {issueCount > 0 && (
          <span
            className={cn(
              "inline-flex shrink-0 items-center gap-1 whitespace-nowrap text-xs font-medium",
              failureTextClass("issue"),
            )}
            data-testid="work-log-issue-count"
          >
            <FailureIcon severity="issue" />
            {formatWorkLogIssueCount(locale, issueCount)}
          </span>
        )}
        {showStatus && (
          <span
            key={statusText}
            className="animate-work-log-status-in min-w-0 shrink truncate text-muted-foreground/80"
            data-testid="work-log-status"
          >
            {statusText}
          </span>
        )}
        <span className="relative h-px min-w-4 flex-1 overflow-hidden bg-border transition-colors group-hover:bg-border/80">
          {isActive && phase && phase !== "queued" && (
            <span className="turn-rule-sheen" data-testid="turn-rule-sheen" />
          )}
        </span>
      </button>

      {pinned != null && <div className="ml-7">{pinned}</div>}
      {attention != null && <div className="ml-7 space-y-3">{attention}</div>}

      <div
        id={bodyId}
        aria-hidden={!open}
        inert={!open}
        onTransitionEnd={(event) => {
          if (event.target === event.currentTarget && !open) setBodyMounted(false);
        }}
        className={cn(
          "grid transition-all duration-200 ease-out",
          open ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          {(open || bodyMounted) && canOpen && (
            <div className="ml-7 space-y-3 pb-1">
              {typeof children === "function" ? children() : children}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Newest entries shown when a work log opens; older ones load in pages. */
export const WORK_LOG_PAGE_SIZE = 200;

interface WorkLogEntriesProps {
  entries: ReactNode[];
}

/**
 * Renders the newest `WORK_LOG_PAGE_SIZE` entries of an opened work log, with
 * a control that reveals earlier ones a page at a time. Keeps a turn with
 * thousands of tool calls cheap to open and to update while it streams.
 */
export function WorkLogEntries({ entries }: WorkLogEntriesProps) {
  const { t } = useLocale();
  const [visibleCount, setVisibleCount] = useState(WORK_LOG_PAGE_SIZE);
  const hiddenCount = Math.max(0, entries.length - visibleCount);

  return (
    <div className="space-y-3">
      {hiddenCount > 0 && (
        <button
          type="button"
          onClick={() => setVisibleCount((current) => current + WORK_LOG_PAGE_SIZE * 5)}
          className="text-xs text-muted-foreground underline-offset-4 transition-colors hover:text-foreground hover:underline"
        >
          {t("show_earlier_steps", { count: hiddenCount })}
        </button>
      )}
      {hiddenCount > 0 ? entries.slice(hiddenCount) : entries}
    </div>
  );
}
