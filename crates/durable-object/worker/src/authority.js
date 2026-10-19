import { Journal, requireValue, stableJson } from "./storage.js";
import { applySessionCommand, translateCommand, sharedReducer } from "./session-command.js";

const identifier = (value) =>
  typeof value === "string" && /^[a-zA-Z0-9_.:-]{1,128}$/.test(value);
const ttl = (value) => {
  requireValue(
    Number.isSafeInteger(value) && value >= 1000 && value <= 60000,
    400,
    "invalid lease duration",
  );
  return value;
};

export class Authority {
  constructor(storage, now = Date.now) {
    this.journal = new Journal(storage, now);
  }

  async handle(operation, body, sessionId, tenant) {
    const j = this.journal;
    if (
      operation === "events" ||
      operation === "snapshot" ||
      operation === "execution" ||
      operation === "event_sequence" ||
      operation === "journal" ||
      operation === "status" ||
      operation === "command" ||
      operation === "receipt" ||
      (operation === "framework" && ["get", "list"].includes(body.operation))
    ) {
      const state = j.read();
      requireValue(
        state.session_id === sessionId && state.tenant === tenant,
        404,
        "session not found",
      );
      if(operation==="execution") return {execution:j.object("session","record"),configuration:j.object("framework","configuration")??{}};
      if(operation==="snapshot") {
        const rejected=j.sql.exec("SELECT key FROM objects WHERE kind='framework' AND key>='rejected/' AND key<'rejected0' ORDER BY key LIMIT 4097").toArray();
        requireValue(rejected.length<=4096,413,"input rejection scan limit exceeded");
        return {binding:state.binding,high_watermark:state.head,configuration:j.object("framework","configuration")??{},execution:j.object("session","record"),rejected_inputs:rejected.map(row=>row.key.slice(9))};
      }
      if(operation==="event_sequence") {
        requireValue(typeof body.event_id==="string" && /^event_[a-f0-9]{32}$/.test(body.event_id),400,"invalid event cursor identity");
        return j.sql.exec("SELECT seq FROM events WHERE json_extract(value,'$.id')=? LIMIT 1",body.event_id).toArray()[0]?.seq??null;
      }
      if (operation === "status") return {...state,now_ms:j.now()};
      if (operation === "command") {
        requireValue(typeof body.id === "string" && /^(approval|approval_submission|resolution|wait_resolution):[a-zA-Z0-9-]{1,64}$/.test(body.id),400,"invalid command lookup");
        const row=j.sql.exec("SELECT fingerprint,value FROM commands WHERE id=?",body.id).toArray()[0];
        return row?{fingerprint:JSON.parse(row.fingerprint),value:JSON.parse(row.value)}:null;
      }
      if (operation === "receipt") {
        requireValue(/^message_[a-f0-9]{32}$/.test(body.message_id), 400, "invalid message identity");
        const row = j.sql.exec("SELECT value FROM commands WHERE id=?", body.message_id).toArray()[0];
        return row ? JSON.parse(row.value) : null;
      }
      if (operation === "framework") {
        requireValue(typeof body.key === "string" && body.key.length <= 512, 400, "invalid Framework receipt key");
        if (body.operation === "list") {
          const rows = j.sql.exec("SELECT key,value FROM objects WHERE kind='framework' AND substr(key,1,?)=? ORDER BY key LIMIT 4097", body.key.length, body.key).toArray();
          requireValue(rows.length <= 4096, 429, "lifecycle receipt scan limit exceeded");
          return rows.map((row) => [row.key, JSON.parse(row.value)]);
        }
        return j.object("framework", body.key);
      }
      let after = body.after ?? 0;
      if (operation === "journal" && body.after == null) {
        const row = j.sql
          .exec("SELECT max(seq) AS seq FROM events WHERE type='turn.started'")
          .one();
        after = Math.max(0, (row.seq ?? 1) - 1);
      }
      const limit =
        operation === "journal" && body.limit === 0 ? 256 : (body.limit ?? 256);
      const page = j.page(state, after, body.through ?? state.head, limit);
      return operation === "journal"
        ? {
            events: page.events,
            has_more: page.has_more,
            head_sequence: state.head || null,
          }
        : page;
    }
    return j.mutate((state) => {
      const command = translateCommand(operation, body, state);
      if (command) return applySessionCommand(j, state, sessionId, tenant, command, sharedReducer);
      requireValue(
        state.session_id === sessionId && state.tenant === tenant,
        404,
        "session not found",
      );
      switch (operation) {
        case "claim": {
          requireValue(identifier(body.holder), 400, "invalid lease holder");
          const duration = ttl(body.ttl_ms);
          if (!state.runnable || state.suspended || state.expires > j.now())
            return null;
          state.holder = body.holder;
          state.fence++;
          state.expires = j.now() + duration;
          state.attempts++;
          state.cancel_reported = state.cancel_inbox_seq !== null;
          state.resume_claimed = state.resume_generation ?? 0;
          return {
            binding: state.binding,
            actor: {
              session_id: sessionId,
              org_id: 1,
              fence: state.fence,
              inbox_seq: state.inbox_seq,
              cancel_inbox_seq: state.cancel_inbox_seq,
              consumed_seq: state.consumed_seq,
              next_input_seq: state.next_input_seq,
              attempts: state.attempts,
            },
          };
        }
        case "renew": {
          j.checkOwner(state, body);
          requireValue(
            Number.isSafeInteger(body.consumed_seq) &&
              body.consumed_seq >= 0 &&
              body.consumed_seq <= state.head,
            400,
            "invalid input cursor",
          );
          state.consumed_seq = Math.max(state.consumed_seq, body.consumed_seq);
          state.expires = j.now() + ttl(body.ttl_ms);
          state.cancel_reported = state.cancel_inbox_seq !== null;
          return {
            inbox_seq: state.inbox_seq,
            cancel_inbox_seq: state.cancel_inbox_seq,
          };
        }
        case "release": {
          j.checkOwner(state, body);
          requireValue(
            Number.isSafeInteger(body.consumed_seq) &&
              body.consumed_seq >= 0 &&
              body.consumed_seq <= state.head,
            400,
            "invalid input cursor",
          );
          state.consumed_seq = Math.max(state.consumed_seq, body.consumed_seq);
          state.holder = null;
          state.expires = 0;
          state.attempts = 0;
          state.failures = 0;
          state.retry_at = 0;
          state.runnable =
            (state.resume_generation ?? 0) > (state.resume_claimed ?? 0) ||
            (state.finalizing?.length ?? 0) > 0 ||
            state.inbox_seq > state.consumed_seq ||
            (state.cancel_inbox_seq !== null && !state.cancel_reported);
          if (state.cancel_reported) state.cancel_inbox_seq = null;
          state.cancel_reported = false;
          return { Released: { still_runnable: state.runnable } };
        }
        case "schedule": {
          j.checkOwner(state,body.lease);
          requireValue(body.wake_at===null || Number.isSafeInteger(body.wake_at) && body.wake_at>=0,400,"invalid activation deadline");
          state.wake_at=body.wake_at;
          return {};
        }
        case "append": {
          j.checkOwner(state, body.lease);
          if (body.conditional && (state.head || null) !== body.expected)
            return { conflict: true, actual: state.head || null, event: null };
          return {
            conflict: false,
            actual: null,
            event: j.append(state, body.event),
          };
        }
        case "resume":
          requireValue(state.binding?.framework == null, 400,
            "Framework sessions require checked retry_execution");
          state.suspended = false;
          state.failures = 0;
          state.retry_at = 0;
          return {};
        case "suspend":
          j.checkOwner(state, body.lease);
          state.suspended = true;
          requireValue(typeof body.reason==="string" && /^[a-z0-9_]{1,128}$/.test(body.reason),400,"invalid suspension diagnostic");
          state.last_error = body.reason;
          return {};
        case "store":
          return this.store(state, body);
        case "framework": {
          j.checkOwner(state, body.lease);
          requireValue(state.binding.framework === 1 && body.operation === "set" && typeof body.key === "string" && body.key.length > 0 && body.key.length <= 512, 400, "invalid Framework receipt");
          requireValue(JSON.stringify(body.value).length <= 1048576, 413, "Framework receipt too large");
          if (body.absent && j.object("framework", body.key) !== null) return false;
          j.put("framework", body.key, body.value);
          if (body.key.startsWith("assignment/")) {
            const target = body.key.slice("assignment/".length);
            const position = state.target_cancels?.[target];
            if (position != null && (!state.open || state.open.turn_id === body.value.turn_id)) {
              state.cancel_inbox_seq = position;
              state.cancel_reported = false;
              state.runnable = true;
            }
          }
          if (body.key.startsWith("outcome/")) {
            const turn = body.key.slice("outcome/".length);
            state.finalizing = (state.finalizing ?? []).filter((id) => id !== turn);
          }
          return true;
        }
        default:
          requireValue(false, 404, "unknown authority operation");
      }
    });
  }

  store(state, body) {
    const j = this.journal;
    const { kind, operation, key, value } = body;
    requireValue(
      ["session", "checkpoint", "value"].includes(kind) &&
        typeof key === "string" &&
        key.length <= 512,
      400,
      "invalid object key",
    );
    if (operation === "get") return j.object(kind, key);
    if (operation === "latest") {
      requireValue(kind === "checkpoint", 400, "invalid checkpoint lookup");
      const [provider, model] = JSON.parse(key);
      const row = j.sql
        .exec(
          "SELECT value FROM objects WHERE kind='checkpoint' AND json_extract(value,'$.provider_type')=? AND json_extract(value,'$.model')=? AND (? IS NULL OR json_extract(value,'$.format_version')=?) ORDER BY json_extract(value,'$.source_sequence') DESC LIMIT 1",
          provider,
          model,
          value,
          value,
        )
        .toArray()[0];
      return row ? JSON.parse(row.value) : null;
    }
    if (operation === "list") {
      return j.sql
        .exec(
          "SELECT key,created AS created_at,updated AS updated_at FROM objects WHERE kind=? ORDER BY key LIMIT 1025",
          kind,
        )
        .toArray()
        .map((entry, _, all) => {
          requireValue(
            all.length <= 1024,
            429,
            "session key capacity exceeded",
          );
          return entry;
        });
    }
    j.checkOwner(state, body.lease);
    const old = j.object(kind, key);
    if (operation === "set") {
      j.put(kind, key, value);
      return {};
    }
    if (operation === "title") {
      requireValue(
        kind === "session" && old && typeof value === "string",
        400,
        "invalid session title",
      );
      old.title = value;
      j.put(kind, key, old);
      return old;
    }
    if (operation === "install") {
      requireValue(
        kind === "checkpoint" &&
          Number.isSafeInteger(value?.source_sequence) &&
          value.session_id === state.session_id,
        400,
        "invalid checkpoint",
      );
      if (old && old.source_sequence >= value.source_sequence) return false;
      j.put(kind, key, value);
      return true;
    }
    if (operation === "take" || operation === "delete") {
      j.sql.exec("DELETE FROM objects WHERE kind=? AND key=?", kind, key);
      return operation === "take" ? old : old !== null;
    }
    requireValue(false, 400, "invalid object operation");
  }
}
