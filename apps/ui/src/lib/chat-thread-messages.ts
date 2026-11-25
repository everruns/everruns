/**
 * Reading the platform's model-facing messages for people.
 *
 * Decisions:
 * - The texts stay written for the model (task ids, the worker's
 *   instructions). A task update becomes a status line without the report
 *   (task-update-marker.tsx); a message that does not match stays the raw
 *   text, folded.
 * - Formats come from `wake_text_for` (crates/core/src/wake_queue.rs) and
 *   `frame_brief` / `frame_relay` (crates/capabilities/src/capabilities/coordination.rs).
 *   Change them together.
 */

export type TaskUpdateKind = "done" | "failed" | "canceled" | "needs_input" | "message";

export type TaskUpdate = {
  kind: TaskUpdateKind;
  title: string;
  /** The task the update is about, so the transcript can tie it to its run card. */
  taskId: string;
  /** The summary, question or detail, without the task id or field labels. */
  body: string;
};

const TERMINAL = /^Task "(.*)" \(([^)\s]+)\) finished: (\w+)\.\n?([\s\S]*)$/;
const AWAITING = /^Task "(.*)" \(([^)\s]+)\) is awaiting input: ([\s\S]*)$/;
const MESSAGE = /^Task "(.*)" \(([^)\s]+)\) sent a message: ([\s\S]*)$/;

const TERMINAL_KIND: Record<string, TaskUpdateKind> = {
  succeeded: "done",
  failed: "failed",
  canceled: "canceled",
};

/** A task wake's text as a headline and a body, or null to show it as is. */
export function parseTaskUpdate(text: string): TaskUpdate | null {
  const terminal = TERMINAL.exec(text);
  if (terminal) {
    const kind = TERMINAL_KIND[terminal[3]];
    if (!kind) return null;
    const summary = terminal[4]
      .split("\n")
      // The result path is a file the model can read; people get the summary.
      .filter((line) => !line.startsWith("- result_path: "))
      .join("\n")
      .replace(/^- summary: /, "")
      .trim();
    return { kind, title: terminal[1], taskId: terminal[2], body: summary };
  }
  const awaiting = AWAITING.exec(text);
  if (awaiting)
    return {
      kind: "needs_input",
      title: awaiting[1],
      taskId: awaiting[2],
      body: awaiting[3].trim(),
    };
  const message = MESSAGE.exec(text);
  if (message)
    return { kind: "message", title: message[1], taskId: message[2], body: message[3].trim() };
  return null;
}

export type CoordinatorMessage =
  | { kind: "assignment"; title: string; body: string }
  | { kind: "relay"; body: string };

// Threads frame what their parent sends ("your parent"); sessions started
// before the Threads capability say "the coordinator".
const ASSIGNMENT = /^New assignment from (?:your parent|the coordinator): (.*)\n\n([\s\S]*)$/;
const RELAY = /^Message from (?:your parent|the coordinator):\n\n([\s\S]*)$/;
/** Starts of the instructions `frame_brief` appends after the brief. */
const BRIEF_INSTRUCTIONS = [
  "\n\nKeep your checklist current with report_progress.",
  "\n\nKeep your checklist current with update_checklist.",
];

/** What a parent sent a thread, without the thread's instructions. */
export function parseCoordinatorMessage(text: string): CoordinatorMessage | null {
  const assignment = ASSIGNMENT.exec(text);
  if (assignment) {
    const rest = assignment[2];
    const end = Math.max(...BRIEF_INSTRUCTIONS.map((marker) => rest.lastIndexOf(marker)));
    const body = (end >= 0 ? rest.slice(0, end) : rest).trim();
    return { kind: "assignment", title: assignment[1].trim(), body };
  }
  const relay = RELAY.exec(text);
  if (relay) return { kind: "relay", body: relay[1].trim() };
  return null;
}
