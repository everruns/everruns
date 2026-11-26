/**
 * One line in the main chat for a subtask wake: why the agent spoke up again.
 *
 * Decisions:
 * - The wake text is the agent's input (task id, field labels, the whole
 *   report). The person reads the agent's reply instead, and the report stays
 *   in the subtask's own thread. So this line carries no report and does not
 *   expand; the raw text is in the info popover for debugging.
 * - When the run card sits right above, the transcript renders no line at all
 *   (chat-message-list.tsx): the card already shows the outcome.
 * - A question stays visible, cut to one line, because the agent's answer
 *   makes no sense without it. Progress messages render nothing.
 */
"use client";

import type { ReactNode } from "react";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useChatWorkspace } from "@/components/chat/chat-workspace-context";
import { runHref } from "@/components/chat/run-card";
import type { SessionTask } from "@/lib/api/types";
import type { TaskUpdate, TaskUpdateKind } from "@/lib/chat-thread-messages";
import { cn } from "@/lib/utils";

type MarkerKind = Exclude<TaskUpdateKind, "message">;

const DOT: Record<MarkerKind, string> = {
  done: "bg-success",
  failed: "bg-destructive",
  canceled: "bg-muted-foreground/40",
  needs_input: "bg-warning",
};

const linkClass =
  "underline decoration-border underline-offset-[3px] transition-colors hover:text-foreground hover:decoration-foreground";

export function TaskUpdateMarker({
  update,
  task,
  trailing,
}: {
  update: TaskUpdate & { kind: MarkerKind };
  /** The task record, when the surface has it: enables jump-to-card and View thread. */
  task?: SessionTask;
  trailing?: ReactNode;
}) {
  const { t } = useLocale();
  const ui = useUiTranslations();
  const workspace = useChatWorkspace();
  const kind = update.kind;

  const showCard = () => {
    const card = document.querySelector<HTMLElement>(
      `[data-run-task-id="${CSS.escape(update.taskId)}"]`,
    );
    card?.scrollIntoView({ behavior: "smooth", block: "center" });
  };

  const viewThread = !task ? null : workspace?.sessionId === task.session_id ? (
    <button type="button" className={linkClass} onClick={() => workspace.openTask(task)}>
      {ui("View thread")}
    </button>
  ) : (
    <a href={runHref(task)} className={linkClass}>
      {ui("Open session")}
    </a>
  );

  return (
    <div
      className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 py-1 text-[13px] text-muted-foreground animate-chat-row-in"
      data-testid="task-update-marker"
      data-kind={kind}
    >
      <span aria-hidden="true" className={cn("h-2 w-2 flex-none", DOT[kind])} />
      {task ? (
        <button
          type="button"
          className={cn("min-w-0 truncate text-foreground", linkClass)}
          aria-label={t("task_marker_show_card", { title: update.title })}
          onClick={showCard}
        >
          {update.title}
        </button>
      ) : (
        <span className="min-w-0 truncate text-foreground">{update.title}</span>
      )}
      <span className={cn("shrink-0", kind === "failed" && "text-destructive")}>
        {t(`task_marker_${kind}` as const)}
      </span>
      {kind === "needs_input" && update.body ? (
        <span className="min-w-0 max-w-[45%] truncate text-foreground" title={update.body}>
          {update.body}
        </span>
      ) : null}
      <span className="h-px min-w-4 flex-1 bg-border" />
      {viewThread}
      {trailing ? <span className="shrink-0">{trailing}</span> : null}
    </div>
  );
}
