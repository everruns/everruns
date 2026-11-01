/**
 * Who wrote a user-role message when no person did.
 *
 * Decisions (knowledge/execution/message-origin.md):
 * - The server normalizes every marker into one `everruns_origin` object on
 *   public message metadata. Earlier rows may still reach a client through
 *   paths that skip the server projection (optimistic state, fixtures), so
 *   this reader maps the earlier markers too: the `"task_wake"` string and
 *   `source: "schedule" | "agent_trigger" | "app_schedule" | "app_webhook"`.
 * - No origin means a person wrote it. Only person-written turns take the
 *   gold user wash; everything else renders as a muted platform row.
 */

export type MessageOriginKind =
  | "schedule"
  | "agent_trigger"
  | "channel_schedule"
  | "webhook"
  | "task_wake"
  | "reaction"
  | "system";

export interface MessageOriginRun {
  number?: number;
  limit?: number;
  ends_at?: string;
  stop_when?: string;
}

export interface MessageOrigin {
  kind: MessageOriginKind;
  source_id?: string;
  name?: string;
  run?: MessageOriginRun;
}

const KINDS: readonly MessageOriginKind[] = [
  "schedule",
  "agent_trigger",
  "channel_schedule",
  "webhook",
  "task_wake",
  "reaction",
  "system",
];

const LEGACY_SOURCES: Record<string, [MessageOriginKind, string]> = {
  schedule: ["schedule", "schedule_id"],
  agent_trigger: ["agent_trigger", "agent_trigger_id"],
  app_schedule: ["channel_schedule", "app_channel_id"],
  app_webhook: ["webhook", "app_channel_id"],
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function count(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function readRun(value: unknown): MessageOriginRun | undefined {
  if (!isRecord(value)) return undefined;
  const run: MessageOriginRun = {
    number: count(value.number),
    limit: count(value.limit),
    ends_at: text(value.ends_at),
    stop_when: text(value.stop_when),
  };
  return Object.values(run).some((v) => v !== undefined) ? run : undefined;
}

/** Read an origin object, as the trace API returns it on a turn. */
export function readOrigin(value: unknown): MessageOrigin | null {
  if (!isRecord(value)) return null;
  const kind = value.kind;
  if (typeof kind !== "string" || !KINDS.includes(kind as MessageOriginKind)) return null;
  return {
    kind: kind as MessageOriginKind,
    source_id: text(value.source_id),
    name: text(value.name),
    run: readRun(value.run),
  };
}

/** The origin of a message, from its metadata. `null` means a person wrote it. */
export function messageOrigin(metadata: unknown): MessageOrigin | null {
  if (!isRecord(metadata)) return null;
  const stored = metadata.everruns_origin;
  if (isRecord(stored)) return readOrigin(stored);
  if (stored === "task_wake") return { kind: "task_wake" };
  const source = typeof metadata.source === "string" ? LEGACY_SOURCES[metadata.source] : undefined;
  if (!source) return null;
  return {
    kind: source[0],
    source_id: text(metadata[source[1]]),
    run: readRun(metadata.schedule_run),
  };
}

/** Message keys for the label of each kind. */
export const ORIGIN_LABEL_KEY = {
  schedule: "scheduled",
  channel_schedule: "scheduled",
  agent_trigger: "origin_trigger",
  webhook: "origin_webhook",
  task_wake: "automatic_update",
  reaction: "origin_reaction",
  system: "origin_system",
} as const satisfies Record<MessageOriginKind, string>;
