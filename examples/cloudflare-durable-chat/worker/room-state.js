// Application state only. Framework owns canonical session journals and recovery.
import { stableJson } from "../../../crates/durable-object/worker/src/storage.js";

export function emptyRoom() {
  return {
    version: 2,
    revision: 0,
    conversations: {},
    receipts: {},
    schedules: {},
    outbox: [],
    searches: {},
  };
}
export function check(condition, message) {
  if (!condition) throw new Error(message);
}
function setReceipt(receipts, key, value) {
  Object.defineProperty(receipts, key, {
    value,
    writable: true,
    enumerable: true,
    configurable: true,
  });
}
export function text(content) {
  if (typeof content === "string") return content;
  return (content ?? [])
    .filter((p) => p.type === "text")
    .map((p) => p.text)
    .join("");
}
export function transcript(events) {
  const messages = [];
  for (const event of events) {
    const data = event.data;
    if (["input.message", "output.message.completed"].includes(event.type)) {
      const raw = data.message;
      const calls = (raw.content ?? [])
        .filter((p) => p.type === "tool_call")
        .map(({ type, ...call }) => call);
      messages.push({
        id: raw.id,
        role: raw.role,
        text: text(raw.content),
        tool_calls: calls,
        sequence: event.sequence,
        raw,
      });
    } else if (event.type === "tool.completed") {
      messages.push({
        id: event.id,
        role: "tool_result",
        text: data.error ?? text(data.result),
        tool_calls: [],
        sequence: event.sequence,
      });
    }
  }
  return messages;
}
export function historySeed(events, sequence = Infinity) {
  const searches = new Set(
    events
      .filter(
        (e) =>
          (e.type === "tool.started" &&
            e.data.tool_name === "search_history") ||
          (e.type === "output.message.completed" &&
            e.data.message.content.some(
              (p) => p.type === "tool_call" && p.name === "search_history",
            )),
      )
      .map((e) => e.context?.turn_id)
      .filter(Boolean),
  );
  const generated = new Set(
    events
      .filter(
        (e) =>
          e.type === "output.message.completed" &&
          searches.has(e.context?.turn_id),
      )
      .map((e) => e.data.message.id),
  );
  return (
    transcript(events.filter((e) => e.sequence <= sequence))
      .filter((m) => m.raw && !m.tool_calls.length && m.text)
      // Seeding intentionally drops execution context. Preserve application
      // provenance so a later fork/import cannot turn search summaries into facts.
      .map((m) =>
        generated.has(m.id)
          ? {
              ...m.raw,
              metadata: { ...m.raw.metadata, chat_history_evidence: false },
            }
          : m.raw,
      )
  );
}
export function legacyRoom(old) {
  const data = {
    ...emptyRoom(),
    revision: old.revision ?? 0,
    schedules: old.schedules ?? {},
  };
  for (const [id, c] of Object.entries(old.conversations ?? {})) {
    const events = c.events.filter((e) => e[2]).map((e) => JSON.parse(e[1]));
    data.conversations[id] = {
      title: c.title,
      parent: c.parent,
      owner: c.owner,
      ready: false,
      imported: true,
    };
    data.outbox.push({
      id: `import:${id}`,
      kind: "create",
      conversation: id,
      seed: historySeed(events),
    });
    // Completed history is imported. An old open tool operation is not blindly
    // repeated under a different runtime; retain its full journal in the backup.
    for (const [n, raw] of c.inbox ?? []) {
      const message = JSON.parse(raw);
      data.outbox.push({
        id: `legacy:${id}:${n}`,
        kind: "send",
        conversation: id,
        message_id: message.id,
        content: message.content,
      });
    }
  }
  for (const [key, [signature, receipt]] of Object.entries(old.receipts ?? {}))
    setReceipt(data.receipts, key, [
      stableJson(JSON.parse(signature)),
      receipt,
    ]);
  return data;
}

export function accept(data, command, session, seed = []) {
  const { request_id: key, ...action } = command;
  check(
    typeof key === "string" && key.length > 0 && key.length <= 200,
    "request_id must contain 1..200 characters",
  );
  const fingerprint = stableJson(action);
  if (Object.hasOwn(data.receipts, key)) {
    check(
      data.receipts[key][0] === fingerprint,
      "request_id already used with different input",
    );
    return data.receipts[key][1];
  }
  check(
    Object.keys(data.receipts).length < 2000,
    "demo room command limit reached",
  );
  let receipt;
  if (action.type === "create" || action.type === "fork") {
    check(
      Object.keys(data.conversations).length < 32,
      "demo room conversation limit reached",
    );
    const parent =
      action.type === "fork" &&
      Object.hasOwn(data.conversations, action.conversation)
        ? data.conversations[action.conversation]
        : null;
    if (action.type === "fork")
      check(parent && !parent.owner, "unknown conversation or search child");
    const title = parent
      ? `Fork of ${parent.title}`
      : String(action.title ?? "").slice(0, 80);
    data.conversations[session] = {
      title,
      parent: parent ? action.conversation : null,
      owner: null,
      ready: false,
    };
    data.outbox.push({
      id: `command:${key}`,
      kind: "create",
      conversation: session,
      seed,
    });
    receipt = { conversation: session };
  } else if (action.type === "send") {
    const c = Object.hasOwn(data.conversations, action.conversation)
      ? data.conversations[action.conversation]
      : null;
    check(c && !c.owner, "unknown conversation or read-only search child");
    check(
      typeof action.text === "string" &&
        action.text.trim() &&
        new TextEncoder().encode(action.text).length <= 8000,
      "message must contain 1..8000 bytes",
    );
    check(
      data.outbox.filter((j) => j.conversation === action.conversation).length <
        32,
      "demo conversation queue limit reached",
    );
    data.outbox.push({
      id: `command:${key}`,
      kind: "send",
      conversation: action.conversation,
      content: [
        {
          type: "text",
          text: `${String(action.username ?? "Guest").slice(0, 40)}: ${action.text}`,
        },
      ],
    });
    receipt = { conversation: action.conversation, queued: true };
  } else if (action.type === "cancel_schedule") {
    check(Object.hasOwn(data.schedules, action.schedule), "unknown schedule");
    data.schedules[action.schedule].active = false;
    receipt = { cancelled: action.schedule };
  } else throw new Error("invalid command");
  setReceipt(data.receipts, key, [fingerprint, receipt]);
  return receipt;
}

export function deliverDue(data, now) {
  for (const [id, schedule] of Object.entries(data.schedules)) {
    if (!schedule.active || schedule.due > now) continue;
    data.outbox.push({
      id: `timer:${id}:${schedule.due}`,
      kind: "send",
      conversation: schedule.conversation,
      content: [
        {
          type: "text",
          text: `[Scheduled instruction] ${schedule.instruction}`,
        },
      ],
    });
    if (schedule.repeat_ms) schedule.due = now + schedule.repeat_ms;
    else schedule.active = false;
  }
}
