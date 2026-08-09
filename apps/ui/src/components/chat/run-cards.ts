/**
 * Runs a chat turn produced, placed back into the transcript.
 *
 * A run is a task the thread's agent started — a delegated subagent, an
 * external agent, a background tool. Tasks carry no turn id, so a run is
 * attributed to the turn whose execution window contains its `created_at`:
 * turns are strictly ordered and non-overlapping within a session, so the last
 * turn that started at or before the task is the turn that started it. A task
 * created outside any turn window (session bootstrap, a schedule firing while
 * nobody was in the thread) is deliberately left unplaced rather than guessed
 * onto the nearest turn.
 */

import type { Event, SessionTask } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";
import { getLastVisibleEventIdByTurn } from "@/components/chat/turn-delimiter";

/** One run card's worth of state. */
export interface ChatRun {
  task: SessionTask;
  /** Tasks the run's own session started underneath it. */
  subagentCount: number;
}

interface TurnWindow {
  turnId: string;
  startedAt: number;
  /** Absent while the turn is still running. */
  endedAt?: number;
}

function getTurnWindows(events: Event[]): TurnWindow[] {
  const windows: TurnWindow[] = [];
  const byTurnId = new Map<string, TurnWindow>();

  for (const event of events) {
    const started = getEventData(event, "turn.started");
    if (started) {
      const window: TurnWindow = { turnId: started.turn_id, startedAt: Date.parse(event.ts) };
      byTurnId.set(started.turn_id, window);
      windows.push(window);
      continue;
    }

    const completed = getEventData(event, "turn.completed");
    if (completed) {
      const window = byTurnId.get(completed.turn_id);
      if (window) window.endedAt = Date.parse(event.ts);
      continue;
    }

    const failed = getEventData(event, "turn.failed");
    if (failed) {
      const window = byTurnId.get(failed.turn_id);
      if (window) window.endedAt = Date.parse(event.ts);
    }
  }

  return windows.filter((window) => !Number.isNaN(window.startedAt));
}

function findTurnId(windows: TurnWindow[], createdAt: number): string | undefined {
  let match: TurnWindow | undefined;
  for (const window of windows) {
    if (window.startedAt > createdAt) break;
    match = window;
  }
  if (!match) return undefined;
  // A finished turn cannot have started a task that appeared after it ended.
  if (match.endedAt != null && createdAt > match.endedAt) return undefined;
  return match.turnId;
}

/**
 * Group runs by the transcript row they render under — the same row the turn
 * divider uses, so a turn's card sits at the end of that turn.
 */
export function assignRunsToEvents(events: Event[], runs: ChatRun[]): Map<string, ChatRun[]> {
  const byEventId = new Map<string, ChatRun[]>();
  if (runs.length === 0) return byEventId;

  const windows = getTurnWindows(events);
  if (windows.length === 0) return byEventId;
  const lastVisibleEventIdByTurn = getLastVisibleEventIdByTurn(events);

  for (const run of runs) {
    const createdAt = Date.parse(run.task.created_at);
    if (Number.isNaN(createdAt)) continue;
    const turnId = findTurnId(windows, createdAt);
    if (!turnId) continue;
    const eventId = lastVisibleEventIdByTurn.get(turnId);
    if (!eventId) continue;
    const existing = byEventId.get(eventId);
    if (existing) existing.push(run);
    else byEventId.set(eventId, [run]);
  }

  return byEventId;
}

/** Wall-clock duration of a run, or null while it has not started. */
export function runDurationMs(task: SessionTask, now: number): number | null {
  if (!task.started_at) return null;
  const startedAt = Date.parse(task.started_at);
  if (Number.isNaN(startedAt)) return null;
  const finishedAt = task.finished_at ? Date.parse(task.finished_at) : now;
  if (Number.isNaN(finishedAt)) return null;
  return Math.max(0, finishedAt - startedAt);
}
