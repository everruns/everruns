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
      if(operation==="event_sequence") {
        requireValue(typeof body.event_id==="string" && /^event_[a-f0-9]{32}$/.test(body.event_id),400,"invalid event cursor identity");
        return j.sql.exec("SELECT seq FROM events WHERE json_extract(value,'$.id')=? LIMIT 1",body.event_id).toArray()[0]?.seq??null;
      }
      if (operation === "status") return {...state,now_ms:j.now()};
      if (operation === "command") {
        requireValue(typeof body.id === "string" && /^approval:[a-zA-Z0-9-]{1,64}$/.test(body.id),400,"invalid command lookup");
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
        state.created_at ??= new Date(j.now()).toISOString();
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
        case "approve": {
          requireValue(identifier(body.command_id),400,"invalid approval command identity");
          const answers=body.approvals;
          requireValue(Array.isArray(answers) && answers.length>0 && answers.length<=256 && new Set(answers.map(answer=>answer.request_id)).size===answers.length,400,"invalid approval batch");
          requireValue(answers.every(answer=>typeof answer?.request_id === "string" && ["allow","reject","allow_always","reject_always"].includes(answer.decision)),400,"invalid approval decision");
          return j.deduplicate(`approval:${body.command_id}`,stableJson(answers),()=>{
            requireValue(Array.isArray(body.prepared) && body.prepared.length===answers.length,400,"invalid prepared approval batch");
            // Validate the complete snapshot before writing. The transaction
            // owns atomicity; Framework Rust owns approval/tool policy.
            for (const [index,answer] of answers.entries()) {
              const call=state.pending.find(call=>call.id===answer.request_id && call.name==="approve_tool_call");
              const request=call?.arguments;
              requireValue(state.open && answer.turn_id===state.open.turn_id && request?.code==="tool_approval_required",409,"approval request is not pending on the targeted turn");
              requireValue(Number.isFinite(Date.parse(request.expires_at)) && Date.parse(request.expires_at)>j.now(),409,"approval request expired");
              const prepared=body.prepared[index];
              requireValue(prepared && stableJson(prepared.expected_request)===stableJson(call) && prepared.not_after_ms===Date.parse(request.expires_at),409,"approval snapshot changed");
              requireValue(typeof prepared.key==="string" && prepared.key.startsWith("tool_approval/") && prepared.key.length<=512 && typeof prepared.record==="string" && prepared.record.length<=32768,400,"invalid approval record");
              requireValue(prepared.result?.tool_call_id===call.id && prepared.result.tool_name===call.name && prepared.spoken?.role==="user",400,"invalid correlated approval answer");
            }
            for (const prepared of body.prepared) {
              j.put("value",prepared.key,prepared.record);
              j.append(state,{type:"tool.completed",ts:new Date(j.now()).toISOString(),session_id:sessionId,context:state.open,data:prepared.result});
              j.append(state,{type:"input.message",ts:new Date(j.now()).toISOString(),session_id:sessionId,context:state.open,data:{message:prepared.spoken}});
            }
            if (!state.pending.length) {state.inbox_seq=state.head;state.resume_generation=(state.resume_generation??0)+1;state.runnable=true;}
            return {};
          });
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
          requireValue(body.results.every(result=>result.tool_name!=="approve_tool_call"),400,"approval requests require an approval command");
          return j.deduplicate(
            `resolution:${body.resolution_id}`,
            stableJson({turn_id:body.turn_id??null,results:body.results}),
            () => {
              requireValue(state.binding.framework!==1 || body.turn_id===state.open?.turn_id,409,"tool resolution targets another turn");
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
                state.resume_generation=(state.resume_generation??0)+1;
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
          const target = body.input_message_id;
          if (state.binding.framework === 1) {
            requireValue(/^message_[a-f0-9]{32}$/.test(target), 400, "Framework cancellation requires an admission target");
          }
          return j.deduplicate(`cancel:${body.command_id}`, stableJson({ target: target ?? null }), () => {
            if (target) {
              const assignment = j.object("framework", `assignment/${target}`);
              const admitted = j.sql.exec("SELECT value FROM commands WHERE id=?", target).toArray()[0];
              requireValue(admitted, 404, "admission not found");
              state.target_cancels ??= {};
              state.target_cancels[target] = state.inbox_seq;
              if (!assignment) return {};
              // A late cancellation of an old admission must never stop a
              // newer turn. Runtime termination and finalization are separate.
              if (state.open?.turn_id !== assignment.turn_id) return {};
            }
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
