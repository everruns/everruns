import { readFileSync } from "node:fs";
import { initSync, session_command, session_command_key } from "../pkg/everruns_durable_object.js";
import { installSessionCommands } from "../src/session-command.js";
initSync({ module: readFileSync(new URL("../pkg/everruns_durable_object_bg.wasm",import.meta.url)) });
installSessionCommands(session_command,session_command_key);
import { DatabaseSync } from "node:sqlite";
import { Authority } from "../src/authority.js";

export const sessionId = `session_${"1".repeat(32)}`;
export const tenant = "test";
export const binding = { agent: "assistant", build_id: "build-one" };

export class Storage {
  constructor() {
    this.db = new DatabaseSync(":memory:");
    this.alarm = null;
    this.failAlarm = false;
    this.queue = Promise.resolve();
    this.sql = {
      exec: (query, ...args) => {
        const rows = this.db.prepare(query).all(...args);
        return {
          [Symbol.iterator]: () => rows[Symbol.iterator](),
          toArray: () => rows,
          one: () => {
            if (rows.length !== 1) throw new Error("expected one row");
            return rows[0];
          },
        };
      },
    };
  }
  async getAlarm() {
    return this.alarm;
  }
  async setAlarm(value) {
    if (this.failAlarm) throw new Error("injected alarm failure");
    this.alarm = value;
  }
  async deleteAlarm() {
    this.alarm = null;
  }
  async transaction(fn) {
    const run = this.queue.then(async () => {
      const previous = this.alarm;
      this.db.exec("BEGIN");
      try {
        const result = await fn(this);
        this.db.exec("COMMIT");
        return result;
      } catch (error) {
        this.db.exec("ROLLBACK");
        this.alarm = previous;
        throw error;
      }
    });
    this.queue = run.catch(() => {});
    return run;
  }
}

export async function fixture(agentBinding=binding) {
  const storage = new Storage();
  let now = 1000000;
  const authority = new Authority(storage, () => now);
  const call = (op, body = {}) => authority.handle(op, body, sessionId, tenant);
  await call("bind", { binding:agentBinding });
  return {
    storage,
    authority,
    call,
    advance: (ms) => {
      now += ms;
    },
  };
}

export function input(index = 1, text = "hello") {
  return {
    type: "input.message",
    ts: new Date().toISOString(),
    session_id: sessionId,
    context: {},
    data: {
      message: {
        id: `message_${index.toString(16).padStart(32, "0")}`,
        role: "user",
        created_at: new Date().toISOString(),
        content: [{ type: "text", text }],
      },
    },
  };
}

export async function claim(f, holder = "worker-one") {
  const c = await f.call("claim", { holder, ttl_ms: 10000 });
  return { holder, fence: c.actor.fence };
}

export const event = (type, turn, data) => ({
  type,
  ts: new Date().toISOString(),
  session_id: sessionId,
  context: { turn_id: turn, input_message_id: input().data.message.id },
  data,
});
export async function start(f, lease, turn = `turn_${"2".repeat(32)}`) {
  await f.call("append", {
    lease,
    event: event("turn.started", turn, {
      turn_id: turn,
      input_message_id: input().data.message.id,
    }),
  });
  return turn;
}
