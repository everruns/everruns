// Presentation rules for rooms in the console (knowledge/runtime-resources/rooms.md).
//
// Decisions:
// - The console inspects rooms that apps make and drives Playground rooms; it is
//   not a chat client. A Playground room is an ordinary room that carries
//   `metadata.playground = true`, so the same API and the same recording apply.
// - Why an agent woke is the `agent.woken` event's `reason`. Events recorded
//   before the server stored one fall back to the message's mentions and
//   thread, and a member's wake setting explains the rest ("everything").
// - An agent set to `relevant` that a check left quiet is an `agent.considered`
//   event with `decision: "quiet"`: the recording shows it listened.
// - Test users in a Playground room are guest members named after virtual
//   users. An org admin posts as them with `as_member`, as a product posts for
//   its own people.

import type { Room, RoomEvent, RoomMember, RoomMessage } from "@/lib/api/rooms";

export const PLAYGROUND_METADATA_KEY = "playground";

export function isPlaygroundRoom(room: Pick<Room, "metadata">): boolean {
  const metadata = room.metadata as Record<string, unknown> | null | undefined;
  return metadata?.[PLAYGROUND_METADATA_KEY] === true;
}

/** Who made a room, as the list shows it. */
export type RoomSource = "playground" | "api";

export function roomSource(room: Pick<Room, "metadata">): RoomSource {
  return isPlaygroundRoom(room) ? "playground" : "api";
}

export interface MemberCounts {
  agents: number;
  people: number;
  outsideAgents: number;
  guests: number;
}

export function countMembers(members: RoomMember[]): MemberCounts {
  const active = members.filter((member) => !member.left);
  const count = (kind: RoomMember["kind"]) =>
    active.filter((member) => member.kind === kind).length;
  return {
    agents: count("agent"),
    people: count("person"),
    outsideAgents: count("outside_agent"),
    guests: count("guest"),
  };
}

/** Why a wake happened, or what stopped it. */
export type WakeReason = "mentioned" | "thread" | "everything" | "relevant" | "limited" | "failed";

const STORED_REASONS: readonly WakeReason[] = ["mentioned", "thread", "everything", "relevant"];

export interface MessageWake {
  memberId: string;
  handle: string;
  reason: WakeReason;
  /** The limit that stopped it, for `limited`. */
  limit?: string;
  sessionId?: string;
  /** The relevance check's confidence, for `relevant`. */
  probability?: number;
}

/** An agent a relevance check left quiet on a message. */
export interface MessageListen {
  memberId: string;
  handle: string;
  /** The check's confidence that the message was for it. */
  probability?: number;
  /** No check could be made: no model answered in time. */
  unavailable: boolean;
}

function eventData(event: RoomEvent): Record<string, unknown> {
  return (event.data as Record<string, unknown> | null | undefined) ?? {};
}

function eventProbability(event: RoomEvent): number | undefined {
  const value = eventData(event).probability;
  return typeof value === "number" ? value : undefined;
}

function eventSessionId(event: RoomEvent): string | undefined {
  const data = event.data as Record<string, unknown> | null | undefined;
  return typeof data?.session_id === "string" ? data.session_id : undefined;
}

function eventLimit(event: RoomEvent): string | undefined {
  const data = event.data as Record<string, unknown> | null | undefined;
  return typeof data?.limit === "string" ? data.limit : undefined;
}

/** The wakes each message caused, keyed by message id, from the room's change feed. */
export function wakesByMessage(
  events: RoomEvent[],
  messages: RoomMessage[],
  members: RoomMember[],
): Map<string, MessageWake[]> {
  const byId = new Map(messages.map((message) => [message.id, message]));
  const memberById = new Map(members.map((member) => [member.id, member]));
  const result = new Map<string, MessageWake[]>();
  // A relevant wake's confidence is on the check that led to it.
  const confidence = new Map<string, number>();
  for (const event of events) {
    const probability = eventProbability(event);
    if (event.type === "agent.considered" && probability !== undefined)
      confidence.set(`${event.message_id}:${event.member_id}`, probability);
  }
  for (const event of events) {
    if (event.type !== "agent.woken" && event.type !== "agent.limited") continue;
    if (!event.message_id || !event.member_id) continue;
    const member = memberById.get(event.member_id);
    const message = byId.get(event.message_id);
    const stored = eventData(event).reason;
    let reason: WakeReason;
    if (event.type === "agent.limited") reason = "limited";
    else if (STORED_REASONS.includes(stored as WakeReason)) reason = stored as WakeReason;
    else if (message?.mentions.some((mention) => mention.id === event.member_id))
      reason = "mentioned";
    else if (message?.thread_id) reason = "thread";
    else reason = "everything";
    const wakes = result.get(event.message_id) ?? [];
    wakes.push({
      memberId: event.member_id,
      handle: member?.handle ?? event.member_id,
      reason,
      limit: event.type === "agent.limited" ? eventLimit(event) : undefined,
      sessionId: eventSessionId(event),
      probability:
        reason === "relevant"
          ? confidence.get(`${event.message_id}:${event.member_id}`)
          : undefined,
    });
    result.set(event.message_id, wakes);
  }
  return result;
}

/** The agents relevance checks left quiet, keyed by message id. */
export function listensByMessage(
  events: RoomEvent[],
  members: RoomMember[],
): Map<string, MessageListen[]> {
  const memberById = new Map(members.map((member) => [member.id, member]));
  const result = new Map<string, MessageListen[]>();
  for (const event of events) {
    if (event.type !== "agent.considered" || !event.message_id || !event.member_id) continue;
    const data = eventData(event);
    if (data.decision !== "quiet") continue;
    const listens = result.get(event.message_id) ?? [];
    listens.push({
      memberId: event.member_id,
      handle: memberById.get(event.member_id)?.handle ?? event.member_id,
      probability: eventProbability(event),
      unavailable: data.error === "unavailable",
    });
    result.set(event.message_id, listens);
  }
  return result;
}

/** The agent sessions each member has in a room, from its wakes. */
export function sessionsByMember(events: RoomEvent[]): Map<string, string[]> {
  const result = new Map<string, string[]>();
  for (const event of events) {
    if (event.type !== "agent.woken" || !event.member_id) continue;
    const sessionId = eventSessionId(event);
    if (!sessionId) continue;
    const sessions = result.get(event.member_id) ?? [];
    if (!sessions.includes(sessionId)) sessions.push(sessionId);
    result.set(event.member_id, sessions);
  }
  return result;
}

/** Room totals the recording shows: messages, agent posts and wakes. */
export function roomTotals(events: RoomEvent[], members: RoomMember[]) {
  const agentIds = new Set(
    members.filter((member) => member.kind === "agent").map((member) => member.id),
  );
  let messages = 0;
  let agentMessages = 0;
  let wakes = 0;
  let limited = 0;
  for (const event of events) {
    if (event.type === "message.posted") {
      messages += 1;
      if (event.member_id && agentIds.has(event.member_id)) agentMessages += 1;
    } else if (event.type === "agent.woken") wakes += 1;
    else if (event.type === "agent.limited") limited += 1;
  }
  return { messages, agentMessages, wakes, limited };
}

/** The handle written in the composer to mention a member. */
export function mentionText(member: Pick<RoomMember, "handle">): string {
  return `@${member.handle}`;
}

/** Handles mentioned in a draft, so the composer can say who will wake. */
export function mentionedHandles(text: string): string[] {
  return [...text.matchAll(/(^|[^\w@])@([a-z0-9][a-z0-9_.-]*)/gi)].map((match) =>
    match[2].toLowerCase(),
  );
}

/** A short handle-friendly name from a person's or virtual user's name. */
export function guestDisplayName(name: string): string {
  return `${name.trim()} (test)`;
}

/** A user id with its `user_` prefix and dashes taken off, so prefixed and raw ids compare. */
export function sameUserId(a?: string | null, b?: string | null): boolean {
  const bare = (id: string) =>
    id
      .replace(/^user_/, "")
      .replaceAll("-", "")
      .toLowerCase();
  return !!a && !!b && bare(a) === bare(b);
}

/** Who a draft will wake, as the composer says it before posting. */
export function draftWakes(
  text: string,
  members: RoomMember[],
  replying: boolean,
): {
  mentioned: RoomMember[];
  everything: RoomMember[];
  listening: RoomMember[];
  thread: boolean;
} {
  const handles = new Set(mentionedHandles(text));
  const agents = members.filter((member) => member.kind === "agent" && !member.left);
  return {
    mentioned: agents.filter(
      (agent) => handles.has(agent.handle.toLowerCase()) && agent.wake !== "never",
    ),
    everything: agents.filter((agent) => agent.wake === "everything"),
    listening: agents.filter(
      (agent) => agent.wake === "relevant" && !handles.has(agent.handle.toLowerCase()),
    ),
    thread: replying,
  };
}
