"use client";

// Cross-session Work view (EVE-756). Lists an org's background tasks from
// GET /v1/tasks, grouped by `root_session_id` into a delegation tree
// (root session → owning sessions → tasks). Chips reuse the shared TaskChip;
// selecting a task opens the shared per-session TaskCard detail. Live updates
// arrive via useOrgTasks (snapshot-then-delta by task_id) — a new task kind
// renders with no frontend change because chips are purely snapshot-driven.

import { useMemo, useState } from "react";
import Link from "next/link";
import { AlertCircle, Waypoints } from "lucide-react";
import { useOrgTasks } from "@/hooks/use-org-tasks";
import { TaskChip } from "@/components/session/session-task-chips";
import { TaskCard } from "@/components/session/task-detail";
import { Skeleton } from "@/components/ui/skeleton";
import type { SessionTask } from "@/lib/api/types";

/** Owning-session subgroup within a delegation tree. */
interface SessionGroup {
  sessionId: string;
  tasks: SessionTask[];
}

/** One delegation tree, rooted at a session, with its tasks grouped by the
 *  owning (descendant) session. */
interface RootGroup {
  rootId: string;
  taskCount: number;
  latest: string;
  sessions: SessionGroup[];
}

function shortId(id: string): string {
  return id.length > 12 ? `…${id.slice(-8)}` : id;
}

/** Group a flat task list into delegation trees: by `root_session_id` (falling
 *  back to the task's own session when it is itself a root), then by owning
 *  session. Roots are ordered by most-recent activity, tasks newest-first. */
function groupByRoot(tasks: SessionTask[]): RootGroup[] {
  const roots = new Map<string, Map<string, SessionTask[]>>();
  for (const task of tasks) {
    const rootId = task.root_session_id ?? task.session_id;
    const bySession = roots.get(rootId) ?? new Map<string, SessionTask[]>();
    const list = bySession.get(task.session_id) ?? [];
    list.push(task);
    bySession.set(task.session_id, list);
    roots.set(rootId, bySession);
  }

  const byCreatedDesc = (a: SessionTask, b: SessionTask) =>
    b.created_at.localeCompare(a.created_at);

  const groups: RootGroup[] = [];
  for (const [rootId, bySession] of roots) {
    const sessions: SessionGroup[] = [];
    let taskCount = 0;
    let latest = "";
    // The root's own tasks sort first; descendant sessions follow.
    const sessionIds = Array.from(bySession.keys()).sort((a, b) => {
      if (a === rootId) return -1;
      if (b === rootId) return 1;
      return a.localeCompare(b);
    });
    for (const sessionId of sessionIds) {
      const list = bySession.get(sessionId)!.slice().sort(byCreatedDesc);
      taskCount += list.length;
      for (const t of list) if (t.updated_at > latest) latest = t.updated_at;
      sessions.push({ sessionId, tasks: list });
    }
    groups.push({ rootId, taskCount, latest, sessions });
  }

  groups.sort((a, b) => b.latest.localeCompare(a.latest));
  return groups;
}

function DelegationTree({
  groups,
  selectedId,
  onSelect,
}: {
  groups: RootGroup[];
  selectedId: string | undefined;
  onSelect: (task: SessionTask) => void;
}) {
  return (
    <div className="space-y-4">
      {groups.map((group) => (
        <div key={group.rootId} className="rounded-lg border border-border bg-card p-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <Link
              href={`/sessions/${group.rootId}`}
              className="flex items-center gap-1.5 text-sm font-medium text-foreground hover:underline"
            >
              <Waypoints className="h-3.5 w-3.5 text-muted-foreground" />
              Session {shortId(group.rootId)}
            </Link>
            <span className="text-xs text-muted-foreground">
              {group.taskCount} {group.taskCount === 1 ? "task" : "tasks"}
            </span>
          </div>
          <div className="space-y-2">
            {group.sessions.map((session) => (
              <div key={session.sessionId}>
                {session.sessionId !== group.rootId ? (
                  <div className="mb-1 pl-1 text-[11px] text-muted-foreground">
                    ↳ via session {shortId(session.sessionId)}
                  </div>
                ) : null}
                <div className="flex flex-wrap items-center gap-1.5">
                  {session.tasks.map((task) => (
                    <TaskChip
                      key={task.id}
                      task={task}
                      selected={task.id === selectedId}
                      onClick={() => onSelect(task)}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

export default function WorkPage() {
  const { data: tasks, isLoading, error } = useOrgTasks();
  const [selectedId, setSelectedId] = useState<string | undefined>(undefined);

  const groups = useMemo(() => groupByRoot(tasks ?? []), [tasks]);
  const selected = useMemo(
    () => (tasks ?? []).find((t) => t.id === selectedId),
    [tasks, selectedId],
  );

  return (
    <div className="flex-1 p-6">
      <div className="mb-6 flex items-center gap-2">
        <Waypoints className="h-5 w-5 text-muted-foreground" />
        <h1 className="text-lg font-medium text-foreground">Work</h1>
        <p className="text-sm text-muted-foreground">
          Tasks across every session, grouped by delegation tree.
        </p>
      </div>

      {isLoading ? (
        <Skeleton className="h-40 w-full" />
      ) : error ? (
        <div className="flex items-center gap-2 text-destructive">
          <AlertCircle className="h-5 w-5" />
          <span>{error instanceof Error ? error.message : "Failed to load tasks"}</span>
        </div>
      ) : groups.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-lg border border-border bg-card py-16 text-muted-foreground">
          <Waypoints className="mb-4 h-12 w-12 opacity-50" />
          <p className="text-lg font-medium">No work yet</p>
          <p className="mt-1 text-sm">
            Subagents, handoffs, background tools, and monitors across your sessions appear here.
          </p>
        </div>
      ) : (
        <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
          <DelegationTree
            groups={groups}
            selectedId={selectedId}
            onSelect={(t) => setSelectedId(t.id)}
          />
          <div className="lg:sticky lg:top-6 lg:self-start">
            {selected ? (
              <TaskCard task={selected} sessionId={selected.session_id} />
            ) : (
              <div className="flex h-40 items-center justify-center rounded-lg border border-dashed border-border text-sm text-muted-foreground">
                Select a task to see its detail.
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
