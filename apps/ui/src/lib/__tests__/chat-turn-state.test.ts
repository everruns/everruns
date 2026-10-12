import type { Event } from "@/lib/api/types";
import {
  buildTurnIndex,
  CLIENT_MESSAGE_ID_METADATA_KEY,
  newClientMessageId,
  reconcilePendingSends,
  rowForPendingSend,
  rowForStoredMessage,
  type PendingSend,
  type StoredMessageRowInput,
} from "../chat-turn-state";

let seq = 0;
const T0 = Date.parse("2026-10-10T10:00:00Z");

function ev(type: string, data: unknown, turnId?: string, atMs?: number): Event {
  seq += 1;
  return {
    id: `ev_${seq}`,
    type,
    ts: new Date(atMs ?? T0 + seq * 1000).toISOString(),
    session_id: "ses_1",
    sequence: seq,
    context: turnId ? { turn_id: turnId } : {},
    data,
  } as unknown as Event;
}

function input(id: string, clientId?: string): Event {
  return ev("input.message", {
    message: {
      id,
      role: "user",
      content: [{ type: "text", text: "yes" }],
      metadata: clientId ? { [CLIENT_MESSAGE_ID_METADATA_KEY]: clientId } : undefined,
    },
  });
}

const started = (turn: string, message: string) =>
  ev("turn.started", { turn_id: turn, input_message_id: message }, turn);

function row(events: Event[], message: Event, extra: Partial<StoredMessageRowInput> = {}) {
  const data = message.data as { message: { id: string } };
  return rowForStoredMessage(buildTurnIndex(events), {
    messageId: data.message.id,
    sequence: message.sequence!,
    storedAtMs: Date.parse(message.ts),
    isLatest: true,
    sessionActive: true,
    hasSteps: () => false,
    ...extra,
  });
}

beforeEach(() => {
  seq = 0;
});

describe("stored message rows", () => {
  it("is starting once stored and before its turn starts", () => {
    const m = input("msg_1");
    expect(row([m], m)).toEqual({
      phase: "starting",
      startedAtMs: Date.parse(m.ts),
    });
  });

  it("goes starting, thinking, working, done with its turn", () => {
    const m = input("msg_1");
    const s = started("turn_1", "msg_1");
    expect(row([m, s], m)?.phase).toBe("starting");
    const r = ev("reason.started", {}, "turn_1");
    expect(row([m, s, r], m)?.phase).toBe("thinking");
    expect(row([m, s, r], m, { hasSteps: () => true })?.phase).toBe("working");
    const done = ev(
      "turn.completed",
      { turn_id: "turn_1", iterations: 2, duration_ms: 4200 },
      "turn_1",
    );
    expect(row([m, s, r, done], m)).toEqual({
      phase: "done",
      turnId: "turn_1",
      durationMs: 4200,
    });
  });

  it("is stopped after a cancel naming its turn", () => {
    const m = input("msg_1");
    const s = started("turn_1", "msg_1");
    const c = ev("turn.cancelled", { turn_id: "turn_1" }, "turn_1", Date.parse(s.ts) + 7000);
    expect(row([m, s, c], m)).toEqual({
      phase: "stopped",
      turnId: "turn_1",
      durationMs: 7000,
    });
  });

  it("is queued while steered into a running turn, then hands over", () => {
    const first = input("msg_1");
    const s = started("turn_1", "msg_1");
    const r1 = ev("reason.started", {}, "turn_1");
    const steered = input("msg_2");
    expect(row([first, s, r1, steered], steered)).toEqual({ phase: "queued" });
    const r2 = ev("reason.started", {}, "turn_1");
    expect(row([first, s, r1, steered, r2], steered)).toBeNull();
  });

  it("has no row once the turn it joined has ended", () => {
    const first = input("msg_1");
    const s = started("turn_1", "msg_1");
    const steered = input("msg_2");
    const done = ev(
      "turn.completed",
      { turn_id: "turn_1", iterations: 1, duration_ms: 10 },
      "turn_1",
    );
    expect(row([first, s, steered, done], steered, { sessionActive: false })).toBeNull();
  });

  it("follows the follow-up turn the server started for it", () => {
    const first = input("msg_1");
    const s1 = started("turn_1", "msg_1");
    const steered = input("msg_2");
    const d1 = ev(
      "turn.completed",
      { turn_id: "turn_1", iterations: 1, duration_ms: 10 },
      "turn_1",
    );
    const s2 = started("turn_2", "msg_2");
    expect(row([first, s1, steered, d1, s2], steered)?.turnId).toBe("turn_2");
  });

  it("has no row for an old message with no turn in view", () => {
    const m = input("msg_1");
    expect(row([m], m, { isLatest: false })).toBeNull();
    expect(row([m], m, { sessionActive: false })).toBeNull();
  });
});

describe("pending sends", () => {
  const base: PendingSend = {
    clientId: "c1",
    text: "yes",
    images: [],
    files: [],
    phase: "sending",
    sentAtMs: T0,
  };

  it("reads sending, then still connecting after 3s", () => {
    expect(rowForPendingSend(base, T0 + 100)).toEqual({ phase: "sending" });
    expect(rowForPendingSend(base, T0 + 3000)).toEqual({ phase: "connecting" });
  });

  it("has no row when not delivered", () => {
    expect(rowForPendingSend({ ...base, phase: "failed" }, T0)).toBeNull();
  });

  it("is queued when steered and starting otherwise", () => {
    const acked = { ...base, phase: "acked" as const, ackAtMs: T0 + 200 };
    expect(rowForPendingSend({ ...acked, delivery: "steered" }, T0)).toEqual({
      phase: "queued",
    });
    expect(rowForPendingSend({ ...acked, delivery: "started" }, T0)).toEqual({
      phase: "starting",
      startedAtMs: T0 + 200,
    });
  });

  it("reconciles by client id or message id, never by text", () => {
    const a = { ...base, clientId: "a" };
    const b = { ...base, clientId: "b", messageId: "msg_9" };
    const c = { ...base, clientId: "c" };
    const events = [input("msg_1", "a"), input("msg_9")];
    expect(reconcilePendingSends([a, b, c], events).map((s) => s.clientId)).toEqual(["c"]);
  });

  it("keeps the same array when nothing matched", () => {
    const pending = [base];
    expect(reconcilePendingSends(pending, [input("msg_1", "other")])).toBe(pending);
  });

  it("mints time-ordered v7 ids", () => {
    const id = newClientMessageId(T0);
    expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(newClientMessageId(T0 + 1) > id).toBe(true);
  });
});
