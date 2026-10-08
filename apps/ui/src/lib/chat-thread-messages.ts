/**
 * Reading the platform's model-facing messages for people.
 *
 * Decisions:
 * - The texts stay written for the model (task ids, the worker's
 *   instructions); the UI shows a readable version and falls back to the raw
 *   text whenever a message does not match.
 * - Formats come from `wake_text_for` (crates/core/src/wake_queue.rs) and
 *   `frame_brief` / `frame_relay` (crates/capabilities/src/capabilities/coordination.rs).
 *   Change them together.
 */

export type TaskUpdateKind = "done" | "failed" | "canceled" | "needs_input" | "message";

export type TaskUpdate = {
  kind: TaskUpdateKind;
  title: string;
  /** The summary, question or detail, without the task id or field labels. */
  body: string;
};

const TERMINAL = /^Task "(.*)" \([^)\s]+\) finished: (\w+)\.\n?([\s\S]*)$/;
const AWAITING = /^Task "(.*)" \([^)\s]+\) is awaiting input: ([\s\S]*)$/;
const MESSAGE = /^Task "(.*)" \([^)\s]+\) sent a message: ([\s\S]*)$/;

const TERMINAL_KIND: Record<string, TaskUpdateKind> = {
  succeeded: "done",
  failed: "failed",
  canceled: "canceled",
};

/** A task wake's text as a headline and a body, or null to show it as is. */
export function parseTaskUpdate(text: string): TaskUpdate | null {
  const terminal = TERMINAL.exec(text);
  if (terminal) {
    const kind = TERMINAL_KIND[terminal[2]];
    if (!kind) return null;
    const summary = terminal[3]
      .split("\n")
      // The result path is a file the model can read; people get the summary.
      .filter((line) => !line.startsWith("- result_path: "))
      .join("\n")
      .replace(/^- summary: /, "")
      .trim();
    return { kind, title: terminal[1], body: summary };
  }
  const awaiting = AWAITING.exec(text);
  if (awaiting)
    return {
      kind: "needs_input",
      title: awaiting[1],
      body: awaiting[2].trim(),
    };
  const message = MESSAGE.exec(text);
  if (message) return { kind: "message", title: message[1], body: message[2].trim() };
  return null;
}

export type CoordinatorMessage =
  | { kind: "assignment"; title: string; body: string }
  | { kind: "relay"; body: string };

const ASSIGNMENT = /^New assignment from the coordinator: (.*)\n\n([\s\S]*)$/;
const RELAY = /^Message from the coordinator:\n\n([\s\S]*)$/;
/** Start of the instructions `frame_brief` appends after the brief. */
const BRIEF_INSTRUCTIONS = "\n\nKeep your checklist current with update_checklist.";

/** What the coordinator sent a thread, without the worker's instructions. */
export function parseCoordinatorMessage(text: string): CoordinatorMessage | null {
  const assignment = ASSIGNMENT.exec(text);
  if (assignment) {
    const rest = assignment[2];
    const end = rest.lastIndexOf(BRIEF_INSTRUCTIONS);
    const body = (end >= 0 ? rest.slice(0, end) : rest).trim();
    return { kind: "assignment", title: assignment[1].trim(), body };
  }
  const relay = RELAY.exec(text);
  if (relay) return { kind: "relay", body: relay[1].trim() };
  return null;
}
