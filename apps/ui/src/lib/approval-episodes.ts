// One approval, as an admin needs to read it.
//
// Soft approval writes two tool completions: `request_approval` (what was
// asked) and `record_approval` (what was written down as approved). The model
// phrases those as different sentences, so the text is not a join key. Within
// one session a grant answers the oldest request that is still open.

import type { AuditLogEntry } from "@/lib/api/audit-logs";
import type { Event } from "@/lib/api/types";
import { getEventData, getTextFromContent, isRecord } from "@/lib/api/types";

export type ApprovalEpisodeStatus = "approved" | "open";

export interface ApprovalAsk {
  action: string;
  question?: string;
  at: string;
  sequence: number;
}

export interface ApprovalGrant {
  action: string;
  detail?: string;
  at: string;
  sequence: number;
  approvedBy: string;
  consentMessageId?: string;
}

export interface ApprovalEpisode {
  id: string;
  status: ApprovalEpisodeStatus;
  /** A grant the session logged with no request still open before it. */
  recordedWithoutRequest: boolean;
  /**
   * An open request with no later user message in the loaded transcript.
   * False when a reply arrived, or when older messages are not loaded and the
   * ask sits outside that window — "still waiting" would be a guess.
   */
  awaitingConsent: boolean;
  ask?: ApprovalAsk;
  grant?: ApprovalGrant;
}

export interface BuildApprovalEpisodesOptions {
  memberNames?: ReadonlyMap<string, string>;
  /** Org audit rows for this session, used when the consent message is not loaded. */
  auditEntries?: readonly AuditLogEntry[];
  /** True when every input message in the session is present in `events`. */
  inputsComplete?: boolean;
  /** Smallest sequence in the loaded transcript window. */
  loadedSinceSequence?: number;
}

interface ToolMark {
  kind: "ask" | "grant";
  eventId: string;
  action: string;
  question?: string;
  detail?: string;
  at: string;
  sequence: number;
  messageId?: string;
}

function payloadString(payload: Record<string, unknown>, key: string): string | undefined {
  const value = payload[key];
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function toolPayload(event: Event): Record<string, unknown> | undefined {
  const data = getEventData(event, "tool.completed");
  if (!data?.success || data.error) return undefined;
  const text = getTextFromContent(data.result ?? []).trim();
  if (!text) return undefined;
  try {
    const parsed: unknown = JSON.parse(text);
    return isRecord(parsed) ? parsed : undefined;
  } catch {
    return undefined;
  }
}

function initiatorUserId(metadata: Record<string, unknown> | undefined): string | undefined {
  const initiator = metadata?.initiator;
  if (!isRecord(initiator) || initiator.type !== "user" || typeof initiator.user_id !== "string") {
    return undefined;
  }
  return initiator.user_id;
}

function messageMetadata(event: Event | undefined): Record<string, unknown> | undefined {
  if (!event) return undefined;
  const input = getEventData(event, "input.message");
  return event.metadata ?? input?.message.metadata ?? undefined;
}

function auditDetail(entry: AuditLogEntry, key: string): string | undefined {
  const value = entry.metadata?.[key];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function byTime(a: ToolMark, b: ToolMark): number {
  if (a.sequence !== b.sequence) return a.sequence - b.sequence;
  const ts = a.at.localeCompare(b.at);
  return ts !== 0 ? ts : a.eventId.localeCompare(b.eventId);
}

function named(userId: string, names: ReadonlyMap<string, string>): string {
  return names.get(userId) || `user ${userId}`;
}

function approvedByLabel(
  messageId: string | undefined,
  inputsById: ReadonlyMap<string, Event>,
  names: ReadonlyMap<string, string>,
  auditActorId: string | undefined,
  auditResolution: string | undefined,
): string {
  const message = messageId ? inputsById.get(messageId) : undefined;
  const userId = initiatorUserId(messageMetadata(message));
  if (userId) return named(userId, names);
  if (message) return "an unattributed initiator";
  if (auditActorId) return named(auditActorId, names);
  if (auditResolution === "unattributed") return "an unattributed initiator";
  return "an unknown actor";
}

function laterUserMessage(ask: ToolMark, inputs: readonly Event[]): boolean {
  return inputs.some((input) => {
    const sequence = input.sequence ?? 0;
    if (sequence > ask.sequence) return true;
    return sequence === ask.sequence && input.ts > ask.at;
  });
}

function stillWaiting(
  ask: ToolMark,
  inputs: readonly Event[],
  options: BuildApprovalEpisodesOptions,
): boolean {
  if (laterUserMessage(ask, inputs)) return false;
  if (options.inputsComplete) return true;
  if (options.loadedSinceSequence === undefined) return false;
  return ask.sequence >= options.loadedSinceSequence;
}

function toolMarks(events: readonly Event[]): ToolMark[] {
  const marks: ToolMark[] = [];
  for (const event of events) {
    const data = getEventData(event, "tool.completed");
    if (!data) continue;
    const kind =
      data.tool_name === "request_approval"
        ? "ask"
        : data.tool_name === "record_approval"
          ? "grant"
          : undefined;
    if (!kind) continue;
    const payload = toolPayload(event);
    if (!payload) continue;
    marks.push({
      kind,
      eventId: event.id,
      action: payloadString(payload, "action") ?? "Critical action",
      question: payloadString(payload, "question"),
      detail: payloadString(payload, "detail"),
      at: event.ts,
      sequence: event.sequence ?? 0,
      messageId:
        kind === "ask"
          ? payloadString(payload, "asked_after_message")
          : payloadString(payload, "approved_in_message"),
    });
  }
  marks.sort(byTime);
  return marks;
}

/**
 * Pair each recorded grant with the request it answers, oldest open request
 * first. Unmatched requests stay open. A grant with nothing waiting is its
 * own approval — category exemptions are recorded that way.
 */
export function buildApprovalEpisodes(
  events: readonly Event[],
  options: BuildApprovalEpisodesOptions = {},
): ApprovalEpisode[] {
  const names = options.memberNames ?? new Map<string, string>();
  const inputs = events.filter((event) => getEventData(event, "input.message"));
  const inputsById = new Map<string, Event>();
  for (const input of inputs) {
    const message = getEventData(input, "input.message");
    if (message?.message.id) inputsById.set(message.message.id, input);
  }

  const auditByMessage = new Map<string, { actorId?: string; resolution?: string }>();
  for (const entry of options.auditEntries ?? []) {
    if (entry.event_type !== "agent.approval.granted") continue;
    const messageId = auditDetail(entry, "approved_in_message");
    if (!messageId || auditByMessage.has(messageId)) continue;
    auditByMessage.set(messageId, {
      actorId: entry.actor_id ?? undefined,
      resolution: auditDetail(entry, "actor_resolution"),
    });
  }

  const open: ToolMark[] = [];
  const episodes: ApprovalEpisode[] = [];

  for (const mark of toolMarks(events)) {
    if (mark.kind === "ask") {
      open.push(mark);
      continue;
    }
    const ask = open.shift();
    const audit = mark.messageId ? auditByMessage.get(mark.messageId) : undefined;
    episodes.push({
      id: mark.eventId,
      status: "approved",
      recordedWithoutRequest: !ask,
      awaitingConsent: false,
      ask: ask
        ? { action: ask.action, question: ask.question, at: ask.at, sequence: ask.sequence }
        : undefined,
      grant: {
        action: mark.action,
        detail: mark.detail,
        at: mark.at,
        sequence: mark.sequence,
        approvedBy: approvedByLabel(
          mark.messageId,
          inputsById,
          names,
          audit?.actorId,
          audit?.resolution,
        ),
        consentMessageId: mark.messageId,
      },
    });
  }

  for (const ask of open) {
    episodes.push({
      id: ask.eventId,
      status: "open",
      recordedWithoutRequest: false,
      awaitingConsent: stillWaiting(ask, inputs, options),
      ask: { action: ask.action, question: ask.question, at: ask.at, sequence: ask.sequence },
    });
  }

  episodes.sort((a, b) => {
    const aKey = a.grant ?? a.ask;
    const bKey = b.grant ?? b.ask;
    const aSequence = aKey?.sequence ?? 0;
    const bSequence = bKey?.sequence ?? 0;
    if (aSequence !== bSequence) return bSequence - aSequence;
    return (bKey?.at ?? "").localeCompare(aKey?.at ?? "");
  });
  return episodes;
}
