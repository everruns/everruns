// Transaction glue only. Commands are interpreted by the shared Rust reducer.
import { requireValue, stableJson } from "./storage.js";

let reducer, receiptKey;
export function installSessionCommands(prepare, key) {
  reducer = prepare;
  receiptKey = key;
}
export function sharedReducer(command, view) {
  requireValue(typeof reducer === "function", 503, "Rust session command implementation unavailable");
  return reducer(command, view);
}

// Compatibility with the reference's transport operations. This translates
// wire values, never validates policy or accepts caller-prepared writes.
export function translateCommand(operation, body, state) {
  switch (operation) {
    case "session_command": return body.command;
    case "bind": return { operation: "create", binding: body.binding };
    case "input":
      requireValue(body.event?.session_id === state.session_id &&
        body.event?.type === "input.message" && !body.event?.context?.turn_id,
        400, "invalid canonical event");
      return { operation: "admit", message: body.event.data?.message, cancelled: body.cancelled ?? false };
    case "configure": return { operation: "configure", id: body.command_id, patch: body.patch };
    case "seed": return { operation: "seed", id: body.command_id, messages: body.events?.map(event => event.data?.message) };
    case "approve":
      return body.submission
        ? { operation: "submit_approvals", id: body.command_id, turn: body.submission.fingerprint.turn_id, decisions: body.submission.fingerprint.decisions }
        : { operation: "approve", id: body.command_id, approvals: body.approvals };
    case "resolve_waits": return { operation: "resolve_waits", id: body.command_id, turn: body.turn_id, approvals: body.approvals, results: body.results };
    case "resolve": return { operation: "resolve", id: body.resolution_id, turn: body.turn_id ?? (state.binding?.framework !== 1 ? state.open?.turn_id : undefined), results: body.results };
    case "cancel": return { operation: "cancel", id: body.command_id, input: body.input_message_id ?? (state.binding?.framework !== 1 ? state.open?.input_message_id : undefined) };
    default: return null;
  }
}

export function applySessionCommand(j, state, session, tenant, command, reduce) {
  requireValue(command && typeof command === "object", 400, "invalid session command");
  if (command.operation !== "create") {
    requireValue(state.session_id === session && state.tenant === tenant, 404, "session not found");
  } else if (state.binding) {
    requireValue(state.session_id === session && state.tenant === tenant, 409, "session binding conflict");
  }
  requireValue(typeof receiptKey === "function", 503, "Rust session command implementation unavailable");
  const identity = JSON.parse(receiptKey(JSON.stringify(command)));
  requireValue(!identity.error, identity.status ?? 400, identity.error);
  const key = identity.key;
  command = identity.command;
  const previous = key ? j.sql.exec("SELECT fingerprint,value FROM commands WHERE id=?", key).toArray()[0] : null;
  const input = command.operation === "cancel" ? command.input : null;
  const assignment = input ? j.object("framework", `assignment/${input}`) : null;
  // These facts and the resulting plan are protected by Journal.mutate's one
  // transaction. Callers cannot provide this view or replace a prepared plan.
  const view = {
    session,
    now_ms: j.now(),
    binding: state.binding ?? null,
    execution_active: state.expires > j.now(),
    suspended: state.suspended,
    suspension_reason: state.last_error ?? null,
    head: state.head,
    open: state.open,
    pending: state.pending,
    configuration: j.object("framework", "configuration") ?? {},
    previous: previous ? { fingerprint: JSON.parse(previous.fingerprint), value: JSON.parse(previous.value) } : null,
    command_count: j.sql.exec("SELECT count(*) AS n FROM commands").one().n,
    queued: command.operation === "admit"
      ? j.sql.exec("SELECT count(*) AS n FROM (SELECT 1 FROM events WHERE type='input.message' AND turn_id IS NULL AND seq>? LIMIT 256)", state.consumed_seq).one().n : 0,
    has_history: command.operation === "seed" && j.sql.exec("SELECT 1 FROM events WHERE type IN ('input.message','output.message.completed') LIMIT 1").toArray().length > 0,
    admitted: !!input && j.sql.exec("SELECT 1 FROM commands WHERE id=?", input).toArray().length > 0,
    assignment: assignment?.turn_id ?? null,
    terminal: !!assignment && j.sql.exec("SELECT 1 FROM events WHERE identity=? LIMIT 1", `terminal:${assignment.turn_id}`).toArray().length > 0,
    rejected: !!input && j.object("framework", `rejected/${input}`) !== null,
  };
  const result = JSON.parse(reduce(JSON.stringify(command), JSON.stringify(view)));
  requireValue(!result.error, result.status ?? 400, result.error);
  const plan = result.plan;
  if (plan.replay) return plan.reply;
  if (plan.control.kind === "bind") {
    state.binding = plan.control.binding;
    state.session_id = session;
    state.tenant = tenant;
    state.created_at ??= new Date(j.now()).toISOString();
  }
  for (const object of plan.objects) j.put(object.kind, object.key, object.value);
  for (const event of plan.events) j.append(state, event);
  switch (plan.control.kind) {
    case "retry_execution":
      state.suspended = false;
      state.runnable = true;
      state.holder = null;
      state.expires = 0;
      state.failures = 0;
      state.retry_at = 0;
      state.last_error = null;
      break;
    case "admit":
      if (plan.control.cancelled) {
        state.target_cancels ??= {};
        state.target_cancels[plan.control.message] = state.head;
      }
      state.inbox_seq = state.head;
      if (state.next_input_seq <= state.consumed_seq) state.next_input_seq = state.head;
      state.runnable = true;
      state.retry_at = 0;
      break;
    case "resume":
      state.inbox_seq = state.head;
      state.resume_generation = (state.resume_generation ?? 0) + 1;
      state.runnable = true;
      break;
    case "cancel":
      state.target_cancels ??= {};
      state.target_cancels[plan.control.input] = state.inbox_seq;
      if (plan.control.interrupt) {
        state.cancel_inbox_seq = state.inbox_seq;
        state.cancel_reported = false;
        state.runnable = true;
      }
      break;
  }
  if (plan.key) {
    j.sql.exec("INSERT INTO commands VALUES (?,?,?)", plan.key, stableJson(plan.fingerprint), JSON.stringify(plan.reply));
  }
  return plan.reply;
}
