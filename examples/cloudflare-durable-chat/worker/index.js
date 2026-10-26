import { DurableObject } from "cloudflare:workers";
import html from "../src/chat.html";
import wasm from "../pkg/everruns_cloudflare_durable_chat_bg.wasm";
import {
  initSync,
  run,
  request as sessionRequest,
  session_command,
  session_command_key,
} from "../pkg/everruns_cloudflare_durable_chat.js";
import {
  SessionAuthority,
  authenticate,
  address,
  readJson,
  failure,
} from "../../../crates/durable-object/worker/src/service.js";
import { installSessionCommands } from "../../../crates/durable-object/worker/src/session-command.js";
import { runActivation } from "../../../crates/durable-object/worker/src/activation.js";
import { objectPort } from "../../../crates/durable-object/worker/src/isolate.js";
import { fetchModel } from "./provider.js";
import {
  accept,
  check,
  deliverDue,
  emptyRoom,
  historySeed,
  legacyRoom,
  transcript,
} from "./room-state.js";
initSync({ module: wasm });
installSessionCommands(session_command, session_command_key);

const MODEL = "meta/muse-spark-1.3-contributor";
const headers = (env) => ({
  authorization: `Bearer ${env.AUTHORITY_TOKEN}`,
  "content-type": "application/json",
});
async function stableId(prefix, value) {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(value),
  );
  return `${prefix}_${Array.from(new Uint8Array(digest))
    .slice(0, 16)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("")}`;
}
function uuid(id) {
  const raw = id.slice(id.indexOf("_") + 1);
  return `${raw.slice(0, 8)}-${raw.slice(8, 12)}-${raw.slice(12, 16)}-${raw.slice(16, 20)}-${raw.slice(20)}`;
}

// No privileged authority route is exposed by the outer Worker. Each bridge is
// scoped to one actor and its room, and credentials remain in the JS host.
export class ChatSession extends SessionAuthority {
  async fetch(request) {
    try {
      await authenticate(request, this.env.AUTHORITY_TOKEN);
      const scope = address(request, this.env);
      check(scope.operation === "engine", "unknown session operation");
      const body = await readJson(request);
      const room = this.ctx.storage.sql
        .exec("SELECT value FROM chat_membership WHERE id=1")
        .toArray()[0]?.value;
      const member = room ? JSON.parse(room) : null;
      if (body.operation === "create") {
        check(/^[a-zA-Z0-9_-]{1,64}$/.test(body.room), "invalid room");
        check(
          !member || (member.room === body.room && member.kind === body.kind),
          "session belongs to another room",
        );
      }
      check(member || body.operation === "create", "session not found");
      const retained = member ?? { room: body.room, kind: body.kind };
      check(
        ["durable-chat", "history-search"].includes(retained.kind),
        "invalid agent",
      );
      if (!retained.model) {
        // Runtime defaults are mutable host configuration. Capture behavior once
        // so retry/reconstruction cannot switch an existing session's model.
        // Pre-pinning memberships used the demo's historical default. Keep that
        // separate from MODEL so an upgrade cannot change their behavior.
        retained.model = member
          ? (this.env.LEGACY_MODEL ?? MODEL)
          : (this.env.MODEL ?? MODEL);
        this.ctx.storage.sql.exec(
          "INSERT INTO chat_membership(id,value) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
          JSON.stringify(retained),
        );
      }
      const reply = JSON.parse(
        await sessionRequest(
          scope.sessionId,
          objectPort(this.authority, {
            tenant: scope.tenant,
            session_id: scope.sessionId,
          }),
          (raw, signal) => fetchModel(this.env, raw, signal),
          (raw) => this.tool(retained, scope.sessionId, raw),
          JSON.stringify(this.configuration(retained, scope.tenant)),
          body.operation,
          JSON.stringify(body.body ?? {}),
        ),
      );
      return Response.json(reply.value, { status: reply.status });
    } catch (error) {
      return failure(error);
    }
  }
  configuration(member, tenant) {
    return { tenant, model: member.model, kind: member.kind };
  }
  constructor(ctx, env) {
    super(ctx, env);
    ctx.storage.sql.exec(
      "CREATE TABLE IF NOT EXISTS chat_membership(id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL)",
    );
  }
  async tool(member, session, raw) {
    const input = JSON.parse(raw);
    check(input.session === session, "tool scope mismatch");
    const response = await roomCall(this.env, member.room, "tool", input);
    return JSON.stringify(response);
  }
  async alarm() {
    await runActivation(this.authority, async (activation) => {
      // Engine validates runtime compatibility and selects the exact retained
      // registration. The host must not replace it with the newest definition.
      const row = this.ctx.storage.sql
        .exec("SELECT value FROM chat_membership WHERE id=1")
        .toArray()[0];
      if (!row) return 409;
      const member = JSON.parse(row.value);
      if (activation.binding.agent !== member.kind) return 409;
      try {
        await run(
          activation.tenant,
          activation.session_id,
          objectPort(this.authority, activation),
          (raw, signal) => fetchModel(this.env, raw, signal),
          (raw) => this.tool(member, activation.session_id, raw),
          member.model ?? this.env.LEGACY_MODEL ?? MODEL,
        );
      } finally {
        // A lost notification affects freshness only. Actor completion is
        // already canonical; reconnect reads Framework state directly.
        await roomCall(this.env, member.room, "changed", {}).catch(() => {});
      }
      return 200;
    });
  }
}

async function sessionCall(env, room, session, kind, operation, body = {}) {
  const tenant = env.TENANT_ID ?? "chat";
  const stub = env.SESSIONS.get(
    env.SESSIONS.idFromName(`${tenant}:${session}`),
  );
  const response = await stub.fetch(
    new Request(
      `https://session.invalid/tenants/${tenant}/sessions/${session}/engine`,
      {
        method: "POST",
        headers: headers(env),
        body: JSON.stringify({ operation, body, room, kind }),
      },
    ),
  );
  if (!response.ok)
    throw new Error(`session ${operation} failed (${response.status})`);
  return response.json();
}
async function roomCall(env, room, operation, body) {
  const stub = env.ROOMS.get(env.ROOMS.idFromName(room));
  const response = await stub.fetch(
    new Request(`https://room.invalid/internal/${room}/${operation}`, {
      method: "POST",
      headers: headers(env),
      body: JSON.stringify(body),
    }),
  );
  if (!response.ok) throw new Error("room operation failed");
  return response.json();
}

export class ChatRoom extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.driving = false;
    ctx.storage.sql.exec(
      "CREATE TABLE IF NOT EXISTS chat(id INTEGER PRIMARY KEY CHECK(id=1),payload TEXT NOT NULL)",
    );
  }
  load() {
    return JSON.parse(
      this.ctx.storage.sql
        .exec("SELECT payload FROM chat WHERE id=1")
        .toArray()[0]?.payload ?? JSON.stringify(emptyRoom()),
    );
  }
  save(data) {
    data.revision++;
    this.ctx.storage.sql.exec(
      "INSERT INTO chat(id,payload) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
      JSON.stringify(data),
    );
    this.changed();
  }
  changed() {
    for (const ws of this.ctx.getWebSockets())
      try {
        ws.send('{"type":"changed"}');
      } catch {}
  }
  async initialize(room) {
    await this.ctx.blockConcurrencyWhile(async () => {
      const name = await this.ctx.storage.get("room_name");
      check(!name || name === room, "room scope mismatch");
      if (!name) await this.ctx.storage.put("room_name", room);
      const data = this.load();
      if (data.version === 2) return;
      await this.ctx.storage.setAlarm(Date.now());
      this.ctx.storage.sql.exec(
        "CREATE TABLE IF NOT EXISTS chat_migration_backup(id INTEGER PRIMARY KEY CHECK(id=1),payload TEXT NOT NULL)",
      );
      this.ctx.storage.sql.exec(
        "INSERT OR IGNORE INTO chat_migration_backup(id,payload) VALUES(1,?)",
        JSON.stringify(data),
      );
      this.save(legacyRoom(data));
    });
  }
  async events(id) {
    const data = this.load();
    check(Object.hasOwn(data.conversations, id), "unknown conversation");
    const c = data.conversations[id];
    if (!c.ready) return [];
    return (
      await sessionCall(
        this.env,
        await this.ctx.storage.get("room_name"),
        id,
        c.owner ? "history-search" : "durable-chat",
        "view",
      )
    ).events;
  }
  async snapshot() {
    const data = this.load(),
      room = await this.ctx.storage.get("room_name");
    const conversations = await Promise.all(
      Object.entries(data.conversations).map(async ([id, c]) => {
        let events = [],
          status = {};
        if (c.ready) {
          const view = await sessionCall(
            this.env,
            room,
            id,
            c.owner ? "history-search" : "durable-chat",
            "view",
          );
          events = view.events;
          status = view.status;
        }
        return {
          id,
          ...c,
          messages: transcript(events).map(({ raw, ...m }) => m),
          activity: events.slice(-40).reverse(),
          execution: {
            phase: status.status ?? "idle",
            build_id: status.build_id,
            compatibility: status.compatibility,
            suspension_reason: status.suspension_reason,
          },
          turn: status.status === "active" ? { phase: status.status } : null,
          queued: data.outbox.filter((j) => j.conversation === id).length,
        };
      }),
    );
    return {
      type: "snapshot",
      revision: data.revision,
      conversations,
      schedules: data.schedules,
      participants: this.ctx
        .getWebSockets()
        .map((ws) => ws.deserializeAttachment())
        .filter(Boolean),
    };
  }
  async command(command) {
    const room = await this.ctx.storage.get("room_name");
    const id = await stableId(
      "session",
      `${room}:command:${command.request_id}`,
    );
    let seed = [];
    if (command.type === "fork") {
      const events = await this.events(command.conversation);
      const boundary = transcript(events).find(
        (m) => m.sequence === command.sequence,
      );
      check(
        boundary?.raw && !boundary.tool_calls.length,
        "fork from a committed user or assistant message",
      );
      seed = historySeed(events, command.sequence);
    }
    return this.ctx.blockConcurrencyWhile(async () => {
      await this.ctx.storage.setAlarm(Date.now());
      const data = this.load();
      const receipt = accept(data, command, id, seed);
      this.save(data);
      return receipt;
    });
  }
  async projectAlarm() {
    await this.ctx.blockConcurrencyWhile(async () => {
      const data = this.load();
      const due = Object.values(data.schedules)
        .filter((s) => s.active)
        .map((s) => s.due);
      if (data.outbox.length)
        due.push(
          Date.now() +
            Math.min(
              60000,
              1000 * 2 ** Math.max(0, (data.delivery_failures ?? 0) - 1),
            ),
        );
      if (due.length)
        await this.ctx.storage.setAlarm(Math.max(Date.now(), Math.min(...due)));
      else await this.ctx.storage.deleteAlarm();
    });
  }
  async alarm() {
    if (this.driving) return this.projectAlarm();
    this.driving = true;
    try {
      let data = this.load();
      deliverDue(data, Date.now());
      this.save(data);
      let delivered = 0;
      while (this.load().outbox.length && delivered++ < 16) {
        // Retain a wake before entering another object's Framework API.
        await this.ctx.storage.setAlarm(Date.now() + 1000);
        const job = this.load().outbox[0],
          c = this.load().conversations[job.conversation];
        const room = await this.ctx.storage.get("room_name"),
          kind = c.owner ? "history-search" : "durable-chat";
        if (job.kind === "create") {
          await sessionCall(this.env, room, job.conversation, kind, "create", {
            creation: { title: c.title, metadata: { room } },
          });
          if (job.seed?.length)
            await sessionCall(this.env, room, job.conversation, kind, "seed", {
              command_id: uuid(await stableId("command", `seed:${job.id}`)),
              messages: job.seed,
            });
        } else {
          await sessionCall(this.env, room, job.conversation, kind, "send", {
            message_id: job.message_id ?? (await stableId("message", job.id)),
            content: job.content,
          });
        }
        data = this.load();
        data.outbox = data.outbox.filter((j) => j.id !== job.id);
        data.delivery_failures = 0;
        if (job.kind === "create")
          data.conversations[job.conversation].ready = true;
        this.save(data);
      }
    } catch (error) {
      const data = this.load();
      data.delivery_failures = Math.min(7, (data.delivery_failures ?? 0) + 1);
      this.save(data);
      console.error("chat delivery paused", error.message);
    } finally {
      this.driving = false;
      await this.projectAlarm();
    }
  }
  async tool(input) {
    const room = await this.ctx.storage.get("room_name"),
      data = this.load();
    const c = data.conversations[input.session];
    check(
      c && typeof input.key === "string" && input.key.length <= 512,
      "invalid tool scope",
    );
    const owner = `${input.session}:${input.key}`,
      args = input.args;
    if (input.name === "search_conversations") {
      check(c.owner, "search tool belongs to a child");
      check(
        typeof args.query === "string" &&
          args.query.trim() &&
          args.query.length <= 200,
        "invalid query",
      );
      const hits = [];
      for (const [id, other] of Object.entries(data.conversations)) {
        if (other.owner) continue;
        const events = await this.events(id);
        const searches = new Set(
          events
            .filter(
              (e) =>
                e.type === "tool.started" &&
                e.data.tool_name === "search_history",
            )
            .map((e) => e.context?.turn_id),
        );
        for (const e of events) {
          if (
            searches.has(e.context?.turn_id) &&
            e.type === "output.message.completed"
          )
            continue;
          if (!["input.message", "output.message.completed"].includes(e.type))
            continue;
          const m = transcript([e])[0];
          if (m.raw.metadata?.chat_history_evidence === false) continue;
          if (m.text.toLowerCase().includes(args.query.toLowerCase()))
            hits.push({
              conversation: id,
              sequence: e.sequence,
              text: m.text.slice(0, 2000),
            });
          if (hits.length >= 12) return hits;
        }
      }
      return hits;
    }
    check(!c.owner, "search children cannot mutate rooms");
    if (input.name === "schedule") {
      return this.ctx.blockConcurrencyWhile(async () => {
        await this.ctx.storage.setAlarm(Date.now());
        const latest = this.load();
        if (!latest.schedules[owner]) {
          check(
            Object.keys(latest.schedules).length < 100,
            "demo schedule limit reached",
          );
          check(
            Number.isInteger(args.delay_seconds) &&
              args.delay_seconds >= 1 &&
              args.delay_seconds <= 604800,
            "invalid delay",
          );
          check(
            args.repeat_seconds == null ||
              (Number.isInteger(args.repeat_seconds) &&
                args.repeat_seconds >= 10 &&
                args.repeat_seconds <= 604800),
            "invalid recurrence",
          );
          check(
            typeof args.instruction === "string" &&
              args.instruction.trim() &&
              args.instruction.length <= 2000,
            "invalid instruction",
          );
          latest.schedules[owner] = {
            conversation: input.session,
            instruction: args.instruction,
            due: Date.now() + args.delay_seconds * 1000,
            repeat_ms: args.repeat_seconds ? args.repeat_seconds * 1000 : null,
            active: true,
          };
          this.save(latest);
        }
        return { schedule_id: owner, due: latest.schedules[owner].due };
      });
    }
    check(
      input.name === "search_history" &&
        typeof args.question === "string" &&
        args.question.length <= 2000,
      "invalid history question",
    );
    const child = await stableId("session", `${room}:search:${owner}`);
    await this.ctx.blockConcurrencyWhile(async () => {
      await this.ctx.storage.setAlarm(Date.now());
      const latest = this.load();
      if (!latest.searches[owner]) {
        check(
          Object.keys(latest.conversations).length < 32,
          "demo conversation limit reached",
        );
        latest.searches[owner] = child;
        latest.conversations[child] = {
          title: "History search",
          parent: input.session,
          owner,
          ready: false,
        };
        latest.outbox.push({
          id: `child:${owner}`,
          kind: "create",
          conversation: child,
          seed: [],
        });
        latest.outbox.push({
          id: `question:${owner}`,
          kind: "send",
          conversation: child,
          content: [
            {
              type: "text",
              text: `Answer with conversation citations: ${args.question}`,
            },
          ],
        });
        this.save(latest);
      }
    });
    // Waiting does not own execution: the child has its own Framework authority
    // and alarm. A replay finds the same child and reconstructible admission.
    const message = await stableId("message", `question:${owner}`);
    const deadline = Date.now() + 90000;
    while (Date.now() < deadline) {
      const current = this.load();
      if (
        current.conversations[child].ready &&
        !current.outbox.some((j) => j.conversation === child)
      ) {
        await sessionCall(this.env, room, child, "history-search", "wait", {
          message_id: message,
        });
        const last = transcript(await this.events(child))
          .filter((m) => m.role === "agent" && m.text)
          .at(-1);
        return {
          answer: last?.text ?? "No history answer.",
          conversation: child,
        };
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    throw new Error("history search pending; retry activation");
  }
  async fetch(request) {
    try {
      const path = new URL(request.url).pathname;
      const internal = path.match(
        /^\/internal\/([a-zA-Z0-9_-]{1,64})\/(tool|changed)$/,
      );
      const external = path.match(
        /^\/rooms\/([a-zA-Z0-9_-]{1,64})\/(state|commands|socket)$/,
      );
      check(internal || external, "unknown room route");
      const [, room, operation] = internal ?? external;
      if (internal) await authenticate(request, this.env.AUTHORITY_TOKEN);
      await this.initialize(room);
      if (operation === "changed") {
        // Session journals live in other DOs. Their committed notifications
        // advance the room watermark so an older in-flight view cannot win.
        this.save(this.load());
        return Response.json({});
      }
      if (operation === "tool")
        return Response.json(await this.tool(await readJson(request)));
      if (operation === "state" && request.method === "GET")
        return Response.json(await this.snapshot());
      if (operation === "commands" && request.method === "POST") {
        const command = await readJson(request);
        check(
          new TextEncoder().encode(JSON.stringify(command)).length <= 16000,
          "command too large",
        );
        return Response.json(await this.command(command));
      }
      if (
        operation === "socket" &&
        request.headers.get("upgrade") === "websocket"
      ) {
        const pair = new WebSocketPair();
        this.ctx.acceptWebSocket(pair[1]);
        pair[1].serializeAttachment({ username: "Guest", conversation: null });
        this.save(this.load());
        pair[1].send(JSON.stringify(await this.snapshot()));
        return new Response(null, { status: 101, webSocket: pair[0] });
      }
      return new Response("not found", { status: 404 });
    } catch (error) {
      return Response.json({ error: error.message }, { status: 400 });
    }
  }
  async webSocketMessage(ws, raw) {
    try {
      check(
        typeof raw === "string" &&
          new TextEncoder().encode(raw).length <= 16000,
        "command too large",
      );
      const command = JSON.parse(raw);
      if (command.type === "select") {
        ws.serializeAttachment({
          username: String(command.username ?? "Guest").slice(0, 40),
          conversation: command.conversation,
        });
        this.save(this.load());
        const snapshot = JSON.stringify(await this.snapshot());
        for (const socket of this.ctx.getWebSockets()) socket.send(snapshot);
      } else {
        const receipt = await this.command(command);
        ws.send(
          JSON.stringify({
            type: "ack",
            request_id: command.request_id,
            receipt,
          }),
        );
      }
    } catch (error) {
      let id;
      try {
        id = JSON.parse(raw).request_id;
      } catch {}
      ws.send(
        JSON.stringify({ type: "error", request_id: id, error: error.message }),
      );
    }
  }
  webSocketClose(ws, code) {
    ws.close([1005, 1006, 1015].includes(code) ? 1000 : code);
    this.save(this.load());
  }
  webSocketError(ws) {
    ws.close(1011, "socket error");
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/") {
      return new Response(html, {
        headers: {
          "content-type": "text/html;charset=utf-8",
          "cache-control": "no-store",
          "referrer-policy": "no-referrer",
          "content-security-policy":
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'",
        },
      });
    }
    const match = url.pathname.match(
      /^\/rooms\/([a-zA-Z0-9_-]{1,64})\/(state|commands|socket)$/,
    );
    if (!match) return new Response("not found", { status: 404 });
    return env.ROOMS.get(env.ROOMS.idFromName(match[1])).fetch(request);
  },
};
