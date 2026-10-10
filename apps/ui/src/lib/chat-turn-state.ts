/**
 * Turn state for the chat's turn status row (knowledge/ui/chat-experience.md).
 *
 * Decisions:
 * - One row per user turn, from Enter to the answer. Before the server acks a
 *   send, the row is driven by the client's pending send; after, by events.
 * - Pending sends are keyed by a client-minted id that the server echoes in
 *   message metadata (`everruns_client_message_id`), never by text, so sending
 *   the same words twice shows two turns.
 * - A message sent while a turn runs is steered into that turn. It shows as
 *   queued until the running turn's next `reason.started` (the step that reads
 *   it), then its row disappears and its work shows in the running turn's log.
 *   A follow-up turn announces itself with `turn.started` naming the message.
 * - Everything here is pure so each state is unit tested.
 */

import type { Controls, Event } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";

/** Metadata key the server stores the client's message id under. */
export const CLIENT_MESSAGE_ID_METADATA_KEY = "everruns_client_message_id";
/** No ack after this long: the row reads "Still connecting…". */
export const STILL_CONNECTING_MS = 3_000;
/** No ack after this long: the send is reported as not delivered. */
export const NOT_DELIVERED_MS = 10_000;

export type MessageDelivery = "started" | "steered" | "resumed" | "duplicate";

export interface PendingSend {
  clientId: string;
  text: string;
  images: Array<{ imageId: string; filename?: string }>;
  files: Array<{ fileId: string; filename?: string }>;
  controls?: Controls;
  addressedParticipantId?: string | null;
  phase: "sending" | "acked" | "failed";
  sentAtMs: number;
  ackAtMs?: number;
  messageId?: string;
  delivery?: MessageDelivery;
  error?: string;
}

export type TurnRowPhase =
  | "sending"
  | "connecting"
  | "queued"
  | "starting"
  | "thinking"
  | "working"
  | "done"
  | "stopped";

export interface TurnRowState {
  phase: TurnRowPhase;
  /** The turn this row belongs to, once the server started it. */
  turnId?: string;
  /** When the "Working for" timer starts (epoch ms): ack, then turn start. */
  startedAtMs?: number;
  /** Final duration for done and stopped rows. */
  durationMs?: number;
}

/** Whether the row is live (sheen, shimmer, ticking timer). */
export function isLiveTurnPhase(phase: TurnRowPhase): boolean {
  return phase !== "done" && phase !== "stopped";
}

/** The client message id a stored user message echoes, if any. */
export function getClientMessageId(event: Event): string | undefined {
  const data = getEventData(event, "input.message");
  const value = data?.message?.metadata?.[CLIENT_MESSAGE_ID_METADATA_KEY];
  return typeof value === "string" ? value : undefined;
}

interface TurnInfo {
  startSeq: number;
  startMs: number;
  inputMessageId: string;
  end?: {
    seq: number;
    kind: "completed" | "failed" | "cancelled";
    durationMs: number;
  };
  reasonSeqs: number[];
}

/** What the event stream says about every turn, built once per events change. */
export interface TurnIndex {
  turns: Map<string, TurnInfo>;
  turnByInputMessageId: Map<string, string>;
  /** Turn ids in start order. */
  order: string[];
}

function seqOf(event: Event): number {
  return typeof event.sequence === "number" && event.sequence >= 0
    ? event.sequence
    : Number.MAX_SAFE_INTEGER;
}

export function buildTurnIndex(events: Event[]): TurnIndex {
  const turns = new Map<string, TurnInfo>();
  const turnByInputMessageId = new Map<string, string>();
  const order: string[] = [];

  for (const event of events) {
    const started = getEventData(event, "turn.started");
    if (started) {
      if (!turns.has(started.turn_id)) {
        turns.set(started.turn_id, {
          startSeq: seqOf(event),
          startMs: Date.parse(event.ts),
          inputMessageId: started.input_message_id,
          reasonSeqs: [],
        });
        order.push(started.turn_id);
      }
      turnByInputMessageId.set(started.input_message_id, started.turn_id);
      continue;
    }

    const turnId = event.context?.turn_id;
    const turn = turnId ? turns.get(turnId) : undefined;
    if (!turn) continue;

    if (event.type === "reason.started") {
      turn.reasonSeqs.push(seqOf(event));
      continue;
    }
    if (turn.end) continue;
    const completed = getEventData(event, "turn.completed");
    if (completed) {
      turn.end = {
        seq: seqOf(event),
        kind: "completed",
        durationMs: completed.duration_ms ?? Math.max(0, Date.parse(event.ts) - turn.startMs),
      };
      continue;
    }
    if (event.type === "turn.failed" || event.type === "turn.cancelled") {
      turn.end = {
        seq: seqOf(event),
        kind: event.type === "turn.failed" ? "failed" : "cancelled",
        durationMs: Math.max(0, Date.parse(event.ts) - turn.startMs),
      };
    }
  }

  return { turns, turnByInputMessageId, order };
}

function rowForTurn(turnId: string, turn: TurnInfo, hasSteps: boolean): TurnRowState {
  if (turn.end) {
    return {
      phase: turn.end.kind === "cancelled" ? "stopped" : "done",
      turnId,
      durationMs: turn.end.durationMs,
    };
  }
  const phase = hasSteps ? "working" : turn.reasonSeqs.length > 0 ? "thinking" : "starting";
  return { phase, turnId, startedAtMs: turn.startMs };
}

export interface StoredMessageRowInput {
  messageId: string;
  sequence: number;
  /** Server time the message was stored (epoch ms). */
  storedAtMs: number;
  /** Whether this is the newest user message in the transcript. */
  isLatest: boolean;
  /** Session status says a turn is running or about to. */
  sessionActive: boolean;
  /** The turn's work log has at least one step. */
  hasSteps: (turnId: string) => boolean;
}

/**
 * The row under a stored user message, or null when it has none: a message
 * steered into a turn that already read it, or an old message whose turn is
 * outside the loaded window.
 */
export function rowForStoredMessage(
  index: TurnIndex,
  input: StoredMessageRowInput,
): TurnRowState | null {
  const ownTurnId = index.turnByInputMessageId.get(input.messageId);
  if (ownTurnId) {
    const turn = index.turns.get(ownTurnId);
    if (turn) return rowForTurn(ownTurnId, turn, input.hasSteps(ownTurnId));
  }

  // A turn that started before this message and had not ended when it was
  // stored is the turn it was steered into.
  for (let i = index.order.length - 1; i >= 0; i -= 1) {
    const turn = index.turns.get(index.order[i])!;
    if (turn.startSeq > input.sequence) continue;
    const coveredByTurn = !turn.end || turn.end.seq > input.sequence;
    if (!coveredByTurn) break;
    if (turn.end) return null;
    const pickedUp = turn.reasonSeqs.some((seq) => seq > input.sequence);
    return pickedUp ? null : { phase: "queued" };
  }

  // Stored, but its turn has not started yet.
  if (input.isLatest && input.sessionActive) {
    return { phase: "starting", startedAtMs: input.storedAtMs };
  }
  return null;
}

/** The row under a send the server has not echoed back as an event yet. */
export function rowForPendingSend(send: PendingSend, nowMs: number): TurnRowState | null {
  if (send.phase === "failed") return null;
  if (send.phase === "sending") {
    return {
      phase: nowMs - send.sentAtMs >= STILL_CONNECTING_MS ? "connecting" : "sending",
    };
  }
  if (send.delivery === "steered") return { phase: "queued" };
  return { phase: "starting", startedAtMs: send.ackAtMs ?? nowMs };
}

/**
 * Drop pending sends the event stream now carries. A send matches its stored
 * event by the server message id from the POST response or by the echoed
 * client id, whichever arrives first.
 */
export function reconcilePendingSends(pending: PendingSend[], events: Event[]): PendingSend[] {
  if (pending.length === 0) return pending;
  const storedClientIds = new Set<string>();
  const storedMessageIds = new Set<string>();
  for (const event of events) {
    if (event.type !== "input.message") continue;
    const clientId = getClientMessageId(event);
    if (clientId) storedClientIds.add(clientId);
    const id = getEventData(event, "input.message")?.message?.id;
    if (id) storedMessageIds.add(id);
  }
  const next = pending.filter(
    (send) =>
      !storedClientIds.has(send.clientId) &&
      !(send.messageId && storedMessageIds.has(send.messageId)),
  );
  return next.length === pending.length ? pending : next;
}

/** A UUID v7 for a new send: time-ordered, so ids sort like the sends. */
export function newClientMessageId(nowMs: number = Date.now()): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  let ts = nowMs;
  for (let i = 5; i >= 0; i -= 1) {
    bytes[i] = ts & 0xff;
    ts = Math.floor(ts / 256);
  }
  bytes[6] = (bytes[6] & 0x0f) | 0x70;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
