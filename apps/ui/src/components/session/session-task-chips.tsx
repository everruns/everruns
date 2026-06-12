"use client";

// SessionTaskChips — compact strip of active task chips shown above the message
// input in the chat view. Renders nothing when there are no active tasks.
//
// Gating: rendered only when the "leased_resources" session feature is present,
// matching the same gate used for the Tasks (resources route) nav tab.

import Link from "next/link";
import { Bot, Cpu, ListTodo, Loader2, Radar } from "lucide-react";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import { cn } from "@/lib/utils";
import type { SessionTask, SessionTaskState } from "@/lib/api/types";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

const ACTIVE_STATES: SessionTaskState[] = ["queued", "running", "awaiting_input"];

function isActiveState(state: SessionTaskState): boolean {
  return ACTIVE_STATES.includes(state);
}

function taskKindIcon(kind: string) {
  if (kind === "subagent" || kind === "external_agent") {
    return <Bot className="h-3 w-3 shrink-0" />;
  }
  if (kind === "background_tool") {
    return <Cpu className="h-3 w-3 shrink-0" />;
  }
  if (kind === "monitor") {
    return <Radar className="h-3 w-3 shrink-0" />;
  }
  return <ListTodo className="h-3 w-3 shrink-0" />;
}

function chipClasses(state: SessionTaskState): string {
  switch (state) {
    case "running":
      // primary — blue tint, matches Badge variant="default"
      return "bg-primary text-primary-foreground border-transparent";
    case "awaiting_input":
      // amber — stands out, matches taskStateBadge in resources/page.tsx
      return "bg-amber-500 text-white border-transparent";
    case "queued":
    default:
      // outline — muted, matches Badge variant="outline"
      return "bg-transparent text-foreground border-border";
  }
}

interface TaskChipProps {
  task: SessionTask;
  tasksHref: string;
}

function TaskChip({ task, tasksHref }: TaskChipProps) {
  const label = task.display_name || task.id;
  // Truncate long names to keep the strip compact.
  const truncated = label.length > 30 ? `${label.slice(0, 28)}…` : label;

  const chip = (
    <Link
      href={tasksHref}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium",
        "transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        chipClasses(task.state),
      )}
      aria-label={`Task: ${label} — ${task.state}`}
    >
      {task.state === "running" && (
        <Loader2 className="h-3 w-3 shrink-0 animate-spin" aria-hidden="true" />
      )}
      {task.state !== "running" && taskKindIcon(task.kind)}
      <span className="max-w-[14ch] truncate">{truncated}</span>
    </Link>
  );

  // Show state_detail as a tooltip when available.
  if (task.state_detail) {
    return (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>{chip}</TooltipTrigger>
          <TooltipContent className="max-w-xs text-xs">{task.state_detail}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }

  return chip;
}

interface SessionTaskChipsProps {
  sessionId: string | undefined;
  /** Base path for this session, e.g. "/sessions/sess_abc" */
  basePath: string;
  /** Whether the leased_resources feature is enabled for this session */
  hasTasksFeature: boolean;
}

export function SessionTaskChips({ sessionId, basePath, hasTasksFeature }: SessionTaskChipsProps) {
  // Gate before mounting the inner component: useSessionTasks requires org
  // context and opens an SSE subscription, so it must not run at all when the
  // feature is off or no session exists.
  if (!hasTasksFeature || !sessionId) {
    return null;
  }
  return <ActiveTaskChips sessionId={sessionId} basePath={basePath} />;
}

function ActiveTaskChips({ sessionId, basePath }: { sessionId: string; basePath: string }) {
  const { data: tasks } = useSessionTasks(sessionId);

  const activeTasks = tasks ? tasks.filter((t) => isActiveState(t.state)) : [];

  // Render nothing (zero vertical space) when there are no active tasks.
  if (activeTasks.length === 0) {
    return null;
  }

  const tasksHref = `${basePath}/resources`;

  return (
    <div
      className="flex flex-wrap items-center gap-1.5 border-t border-border/60 bg-muted/30 px-3 py-1.5"
      role="region"
      aria-label="Active tasks"
    >
      {activeTasks.map((task) => (
        <TaskChip key={task.id} task={task} tasksHref={tasksHref} />
      ))}
    </div>
  );
}
