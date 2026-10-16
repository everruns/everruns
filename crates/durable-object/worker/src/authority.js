import { Journal, requireValue, stableJson } from "./storage.js";

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
      operation === "journal" ||
      operation === "status"
    ) {
      const state = j.read();
      requireValue(
        state.session_id === sessionId && state.tenant === tenant,
        404,
        "session not found",
      );
      if (operation === "status") return state;
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
      if (operation === "bind") {
        requireValue(
          identifier(body.binding?.agent) && identifier(body.binding?.build_id),
          400,
          "invalid agent build binding",
        );
        if (state.binding)
          requireValue(
            stableJson(state.binding) === stableJson(body.binding) &&
              state.session_id === sessionId &&
              state.tenant === tenant,
            409,
            "session belongs to another agent build",
          );
        state.binding = body.binding;
        state.session_id = sessionId;
        state.tenant = tenant;
        return {};
      }
      requireValue(
        state.session_id === sessionId && state.tenant === tenant,
        404,
        "session not found",
      );
      switch (operation) {
        case "input": {
          const event = body.event;
          requireValue(
            event?.type === "input.message" &&
              !event.context?.turn_id &&
              event.data?.message?.role === "user" &&
              /^message_[a-f0-9]{32}$/.test(event.data?.message?.id),
            400,
            "invalid user input",
          );
          const {
            created_at: _created,
            updated_at: _updated,
            ...message
          } = event.data.message;
          return j.deduplicate(message.id, stableJson(message), () => {
            const pending = j.sql
              .exec(
                "SELECT count(*) AS n FROM events WHERE type='input.message' AND turn_id IS NULL AND seq>?",
                state.consumed_seq,
              )
              .one().n;
            requireValue(pending < 256, 429, "session inbox is full");
            const stored = j.append(state, event);
            state.inbox_seq = stored.sequence;
            if (state.next_input_seq <= state.consumed_seq)
              state.next_input_seq = stored.sequence;
            state.runnable = true;
            state.retry_at = 0;
            return { message_id: message.id, sequence: stored.sequence };
          });
        }
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
            state.inbox_seq > state.consumed_seq ||
            (state.cancel_inbox_seq !== null && !state.cancel_reported);
          if (state.cancel_reported) state.cancel_inbox_seq = null;
          state.cancel_reported = false;
          return { Released: { still_runnable: state.runnable } };
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
        case "resolve": {
          requireValue(
            identifier(body.resolution_id),
            400,
            "invalid resolution identity",
          );
          requireValue(
            Array.isArray(body.results) &&
              body.results.length > 0 &&
              body.results.length <= 256,
            400,
            "invalid tool results",
          );
          return j.deduplicate(
            `resolution:${body.resolution_id}`,
            stableJson(body.results),
            () => {
              requireValue(
                state.open && state.pending.length,
                409,
                "session has no pending client tools",
              );
              requireValue(
                new Set(body.results.map((r) => r.tool_call_id)).size ===
                  body.results.length,
                400,
                "duplicate tool results",
              );
              for (const result of body.results) {
                requireValue(
                  state.pending.some(
                    (call) =>
                      call.id === result.tool_call_id &&
                      call.name === result.tool_name,
                  ),
                  409,
                  "tool result does not match a pending call",
                );
                j.append(state, {
                  type: "tool.completed",
                  ts: new Date(j.now()).toISOString(),
                  session_id: sessionId,
                  context: state.open,
                  data: result,
                });
              }
              // Partial answers stay parked until every requested call is resolved.
              if (!state.pending.length) {
                state.inbox_seq = state.head;
                state.runnable = true;
              }
              return {};
            },
          );
        }
        case "cancel": {
          requireValue(
            identifier(body.command_id),
            400,
            "invalid cancellation identity",
          );
          return j.deduplicate(`cancel:${body.command_id}`, "cancel", () => {
            if (state.open || state.runnable) {
              state.cancel_inbox_seq = state.inbox_seq;
              state.cancel_reported = false;
              state.runnable = true;
            }
            return {};
          });
        }
        case "resume":
          state.suspended = false;
          state.failures = 0;
          state.retry_at = 0;
          return {};
        case "suspend":
          j.checkOwner(state, body.lease);
          state.suspended = true;
          state.last_error = "host_configuration_error";
          return {};
        case "store":
          return this.store(state, body);
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
