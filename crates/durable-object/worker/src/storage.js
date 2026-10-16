// Canonical session state and control mutations share one SQLite transaction.
// No agent decisions live here: the native shared driver owns turn semantics.

export class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

export function requireValue(condition, status, message) {
  if (!condition) throw new HttpError(status, message);
}

export function stableJson(value) {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${stableJson(value[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

export class Journal {
  constructor(storage, now = Date.now) {
    this.storage = storage;
    this.sql = storage.sql;
    this.now = now;
    this.sql.exec(
      "CREATE TABLE IF NOT EXISTS control (id INTEGER PRIMARY KEY CHECK (id=1), value TEXT NOT NULL)",
    );
    this.sql.exec(
      "CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY, type TEXT NOT NULL, turn_id TEXT, identity TEXT UNIQUE, value TEXT NOT NULL)",
    );
    this.sql.exec(
      "CREATE INDEX IF NOT EXISTS events_turn_start ON events(type,seq)",
    );
    this.sql.exec(
      "CREATE TABLE IF NOT EXISTS commands (id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, value TEXT NOT NULL)",
    );
    this.sql.exec(
      "CREATE TABLE IF NOT EXISTS objects (kind TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, created TEXT NOT NULL, updated TEXT NOT NULL, PRIMARY KEY(kind,key))",
    );
  }

  read() {
    const row = this.sql
      .exec("SELECT value FROM control WHERE id=1")
      .toArray()[0];
    return row
      ? JSON.parse(row.value)
      : {
          version: 1,
          head: 0,
          inbox_seq: 0,
          consumed_seq: 0,
          next_input_seq: 0,
          runnable: false,
          holder: null,
          fence: 0,
          expires: 0,
          attempts: 0,
          cancel_inbox_seq: null,
          cancel_reported: false,
          open: null,
          pending: [],
          suspended: false,
          failures: 0,
        };
  }

  write(state) {
    this.sql.exec(
      "INSERT INTO control VALUES (1,?) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
      JSON.stringify(state),
    );
  }

  // Use the async storage transaction for SQL plus the Alarms API. In
  // particular, do not put a setAlarm Promise inside transactionSync.
  async mutate(fn) {
    return this.storage.transaction(async () => {
      const state = this.read();
      requireValue(
        state.version === 1,
        409,
        "unsupported authority state version",
      );
      const result = fn(state);
      this.write(state);
      if (state.runnable && !state.suspended) {
        const desired =
          state.holder && state.expires > this.now()
            ? state.expires
            : Math.max(this.now() + 10, state.retry_at || 0);
        const armed = await this.storage.getAlarm();
        if (armed === null || desired < armed)
          await this.storage.setAlarm(desired);
      }
      return result;
    });
  }

  checkOwner(state, lease) {
    requireValue(
      lease &&
        typeof lease.holder === "string" &&
        state.holder === lease.holder &&
        state.fence === lease.fence &&
        state.expires > this.now(),
      412,
      "execution lease lost",
    );
  }

  deduplicate(id, fingerprint, apply) {
    requireValue(
      typeof id === "string" && id.length > 0 && id.length <= 128,
      400,
      "invalid command identity",
    );
    const row = this.sql
      .exec("SELECT fingerprint,value FROM commands WHERE id=?", id)
      .toArray()[0];
    if (row) {
      requireValue(
        row.fingerprint === fingerprint,
        409,
        "command identity reused with different input",
      );
      return JSON.parse(row.value);
    }
    const count = this.sql.exec("SELECT count(*) AS n FROM commands").one().n;
    requireValue(count < 100000, 429, "session command capacity reached");
    const result = apply();
    this.sql.exec(
      "INSERT INTO commands VALUES (?,?,?)",
      id,
      fingerprint,
      JSON.stringify(result),
    );
    return result;
  }

  append(state, request) {
    requireValue(
      request &&
        request.session_id === state.session_id &&
        typeof request.type === "string" &&
        typeof request.context === "object" &&
        request.context !== null &&
        typeof request.data === "object" &&
        request.data !== null,
      400,
      "invalid canonical event",
    );
    requireValue(
      !request.type.endsWith(".delta") &&
        request.type !== "reason.thinking.delta",
      400,
      "ephemeral event cannot enter the journal",
    );
    const turn = request.context.turn_id ?? null;
    let identity = null;
    if (
      [
        "turn.completed",
        "turn.failed",
        "turn.cancelled",
        "turn.sealed",
      ].includes(request.type)
    )
      identity = `terminal:${turn}`;
    if (["tool.started", "tool.completed"].includes(request.type)) {
      const call = request.data.tool_call_id ?? request.data.tool_call?.id;
      requireValue(
        turn && typeof call === "string",
        400,
        "tool event lacks operation identity",
      );
      identity = `${request.type}:${turn}:${call}`;
    }
    if (identity) {
      const existing = this.sql
        .exec("SELECT value FROM events WHERE identity=?", identity)
        .toArray()[0];
      if (existing) {
        const event = JSON.parse(existing.value);
        requireValue(
          event.type === request.type &&
            stableJson(event.data) === stableJson(request.data),
          409,
          "operation outcome conflicts with its committed record",
        );
        return event;
      }
    }
    requireValue(
      state.head < 2147483647,
      429,
      "session sequence capacity reached",
    );
    const event = {
      ...request,
      id: `event_${crypto.randomUUID().replaceAll("-", "")}`,
      sequence: ++state.head,
    };
    this.sql.exec(
      "INSERT INTO events VALUES (?,?,?,?,?)",
      event.sequence,
      event.type,
      turn,
      identity,
      JSON.stringify(event),
    );
    if (event.type === "turn.started") {
      requireValue(!state.open, 409, "session already has an open turn");
      state.open = {
        turn_id: event.data.turn_id,
        input_message_id: event.data.input_message_id,
      };
    }
    if (identity?.startsWith("terminal:")) {
      requireValue(
        state.open?.turn_id === turn ||
          (!state.open && event.type === "turn.sealed"),
        409,
        "terminal event belongs to another turn",
      );
      state.open = null;
      state.pending = [];
    }
    if (event.type === "tool.call_requested")
      state.pending = event.data.tool_calls ?? [];
    if (event.type === "tool.completed")
      state.pending = state.pending.filter(
        (call) => call.id !== event.data.tool_call_id,
      );
    return event;
  }

  page(state, after, through, limit) {
    requireValue(
      Number.isSafeInteger(after) &&
        after >= 0 &&
        Number.isSafeInteger(through) &&
        through >= after &&
        through <= state.head &&
        Number.isSafeInteger(limit) &&
        limit > 0 &&
        limit <= 1024,
      400,
      "invalid event cursor or limit",
    );
    const rows = this.sql.exec(
      "SELECT value FROM events WHERE seq>? AND seq<=? ORDER BY seq LIMIT ?",
      after,
      through,
      limit + 1,
    );
    let bytes = 0;
    const events = [];
    let hasMore = false;
    // Iterate lazily: materializing 1025 large events can exceed isolate memory.
    for (const row of rows) {
      if (events.length === limit) {
        hasMore = true;
        break;
      }
      bytes += new TextEncoder().encode(row.value).length;
      if (bytes > 2 * 1024 * 1024 && events.length) {
        hasMore = true;
        break;
      }
      events.push(JSON.parse(row.value));
    }
    return { events, high_watermark: through, has_more: hasMore };
  }

  object(kind, key) {
    const row = this.sql
      .exec("SELECT value FROM objects WHERE kind=? AND key=?", kind, key)
      .toArray()[0];
    return row ? JSON.parse(row.value) : null;
  }

  put(kind, key, value) {
    const count = this.sql
      .exec("SELECT count(*) AS n FROM objects WHERE kind=?", kind)
      .one().n;
    requireValue(
      count < 1024 || this.object(kind, key) !== null,
      429,
      "session object capacity reached",
    );
    const created = new Date(this.now()).toISOString();
    this.sql.exec(
      "INSERT INTO objects VALUES (?,?,?,?,?) ON CONFLICT(kind,key) DO UPDATE SET value=excluded.value,updated=excluded.updated",
      kind,
      key,
      JSON.stringify(value),
      created,
      created,
    );
  }
}
