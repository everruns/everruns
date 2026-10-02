// The durable half of a serve app on celld: one Durable Object (a cell)
// keeps a serve container's state across container loss, node loss and
// moves between nodes.
//
// Decisions:
// - The container is a cache. Its disk dies with a move, a node restart or a
//   reset, so everything that must survive lives in this cell's SQLite,
//   which celld replicates and fails over.
// - Snapshot plus journal. When the app is idle the cell pulls a snapshot
//   (`GET /celld/snapshot`: serve's databases and workspace as a tar). Every
//   mutating request after it is journaled here before it reaches the
//   container. A fresh container (a new `boot_id` in `GET /celld/state`) is
//   restored from the snapshot and the journal is replayed in order, so a
//   turn that a crash interrupted runs again.
// - One writer. Every mutating request, restore, replay and snapshot runs
//   under one in-cell lock, so a snapshot and the journal never disagree
//   about whether a request is in it. Streams (SSE, AG-UI) only hold the
//   lock until their response starts.
// - `POST /v1/sessions` is not journaled: serve picks the session id, so a
//   replay would mint a different one. It is acknowledged only after a
//   snapshot holds it. Requests on an existing session (messages,
//   approvals, answers, cancel, AG-UI runs on a `threadId`) replay safely.
// - Turn-level durability: a replayed turn calls the model again and may
//   run its tools again. Side-effecting tools need their own idempotency.
// - Snapshots are kept as 1 MiB rows in this cell's SQLite, so the flip
//   from one snapshot to the next commits atomically with the journal
//   truncation. The whole snapshot passes through the isolate's heap
//   (128 MB by default), which bounds the state of one cell.

const CHUNK = 1 << 20;
const IDLE_POLL_MS = 250;
const CREATE_WAIT_MS = 30_000;
const REPLAY_WAIT_MS = 15 * 60_000;
const FLUSH_RETRY_S = 2;

export class Cell {
  /**
   * @param {object} deps
   * @param {DurableObjectStorage} deps.storage the cell's storage
   * @param {(request: Request) => Promise<Response>} deps.upstream reaches the container
   * @param {(seconds: number) => Promise<unknown>} deps.later runs `flush()` after a delay
   */
  constructor({ storage, upstream, later, log = console.log }) {
    this.storage = storage;
    this.sql = storage.sql;
    this.upstream = upstream;
    this.later = later;
    this.log = log;
    this.queue = Promise.resolve();
    this.sql.exec(`CREATE TABLE IF NOT EXISTS serve_meta (key TEXT PRIMARY KEY, value TEXT)`);
    this.sql.exec(
      `CREATE TABLE IF NOT EXISTS serve_snapshot (seq INTEGER, idx INTEGER, data BLOB, PRIMARY KEY (seq, idx))`,
    );
    this.sql.exec(
      `CREATE TABLE IF NOT EXISTS serve_journal (seq INTEGER PRIMARY KEY AUTOINCREMENT, method TEXT, path TEXT, content_type TEXT, body BLOB)`,
    );
  }

  /** A request from the Worker, with the path the container should see. */
  async handle(request) {
    const url = new URL(request.url);
    if (url.pathname.startsWith("/celld/")) {
      // The container's control routes are the cell's alone.
      return new Response("not found\n", { status: 404 });
    }
    if (url.pathname === "/_cell/state") {
      return Response.json(this.status());
    }
    const method = request.method;
    if (method === "GET" || method === "HEAD") {
      await this.exclusive(() => this.ensureCurrent());
      return this.upstream(new Request(url, request));
    }
    const body = await request.arrayBuffer();
    const contentType = request.headers.get("content-type") ?? "";
    const create = method === "POST" && url.pathname === "/v1/sessions";
    return this.exclusive(async () => {
      await this.ensureCurrent();
      if (!create) {
        this.sql.exec(
          `INSERT INTO serve_journal (method, path, content_type, body) VALUES (?, ?, ?, ?)`,
          method,
          url.pathname + url.search,
          contentType,
          body,
        );
      }
      const response = await this.upstream(
        new Request(url, { method, headers: request.headers, body }),
      );
      if (create) {
        if (response.ok && !(await this.checkpointWhenIdle(CREATE_WAIT_MS))) {
          // Still answered: the session exists. It is only lost if the
          // container dies before the next snapshot.
          this.log("serve-celld: session created while busy; snapshot deferred");
          await this.later(FLUSH_RETRY_S);
          const headers = new Headers(response.headers);
          headers.set("x-celld-durable", "pending");
          return new Response(response.body, { status: response.status, headers });
        }
      } else {
        await this.later(1);
      }
      return response;
    });
  }

  /** Snapshot once the app is idle; the scheduled callback. */
  async flush() {
    return this.exclusive(async () => {
      await this.ensureCurrent();
      if (this.journalLength() === 0 && !this.meta("dirty")) return;
      if (!(await this.checkpoint())) await this.later(FLUSH_RETRY_S);
    });
  }

  status() {
    return {
      boot_id: this.meta("boot_id"),
      snapshot_seq: Number(this.meta("snapshot_seq") ?? 0),
      snapshot_bytes: Number(
        this.sql
          .exec(`SELECT COALESCE(SUM(LENGTH(data)), 0) AS n FROM serve_snapshot`)
          .one().n,
      ),
      journal: this.journalLength(),
      restores: Number(this.meta("restores") ?? 0),
      replayed: Number(this.meta("replayed") ?? 0),
    };
  }

  // Runs `fn` after every earlier exclusive section, whether it failed or not.
  exclusive(fn) {
    const run = this.queue.then(fn, fn);
    this.queue = run.catch(() => {});
    return run;
  }

  meta(key) {
    const rows = this.sql.exec(`SELECT value FROM serve_meta WHERE key = ?`, key).toArray();
    return rows.length ? rows[0].value : null;
  }

  setMeta(key, value) {
    if (value === null) {
      this.sql.exec(`DELETE FROM serve_meta WHERE key = ?`, key);
    } else {
      this.sql.exec(
        `INSERT INTO serve_meta (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value`,
        key,
        String(value),
      );
    }
  }

  journalLength() {
    return Number(this.sql.exec(`SELECT COUNT(*) AS n FROM serve_journal`).one().n);
  }

  async containerState() {
    const response = await this.upstream(new Request("http://container/celld/state"));
    if (!response.ok) throw new Error(`celld/state answered ${response.status}`);
    return response.json();
  }

  // Bring a fresh container up to date: restore, then replay the journal.
  async ensureCurrent() {
    const state = await this.containerState();
    if (state.boot_id === this.meta("boot_id")) return;
    if (state.booted) {
      // Something reached the app before the cell did; its state cannot be
      // replaced any more. Adopt it rather than fail every request.
      this.log(`serve-celld: container ${state.boot_id} booted before restore; adopting it`);
      this.setMeta("boot_id", state.boot_id);
      this.setMeta("dirty", 1);
      return;
    }
    const seq = this.meta("snapshot_seq");
    if (seq !== null) {
      const parts = this.sql
        .exec(`SELECT data FROM serve_snapshot WHERE seq = ? ORDER BY idx`, Number(seq))
        .toArray()
        .map((row) => new Uint8Array(row.data));
      const response = await this.upstream(
        new Request("http://container/celld/restore", {
          method: "PUT",
          headers: { "content-type": "application/x-tar" },
          body: concat(parts),
        }),
      );
      if (!response.ok) {
        throw new Error(`celld/restore answered ${response.status}: ${await response.text()}`);
      }
      this.setMeta("restores", Number(this.meta("restores") ?? 0) + 1);
    }
    const journal = this.sql
      .exec(`SELECT seq, method, path, content_type, body FROM serve_journal ORDER BY seq`)
      .toArray();
    for (const entry of journal) {
      const headers = entry.content_type ? { "content-type": entry.content_type } : {};
      const response = await this.upstream(
        new Request(`http://container${entry.path}`, {
          method: entry.method,
          headers,
          body: entry.body && entry.body.byteLength ? entry.body : undefined,
        }),
      );
      // Drain: an AG-UI run ends with its stream.
      await response.arrayBuffer();
      if (!response.ok) {
        this.log(`serve-celld: replay of ${entry.method} ${entry.path} answered ${response.status}`);
      }
      // The next request may depend on this one's turn (an approval, say).
      await this.waitIdle(REPLAY_WAIT_MS);
    }
    this.setMeta("replayed", Number(this.meta("replayed") ?? 0) + journal.length);
    this.setMeta("boot_id", state.boot_id);
    if (seq !== null || journal.length) {
      this.log(
        `serve-celld: restored container ${state.boot_id} from snapshot ${seq ?? "none"} and replayed ${journal.length} request(s)`,
      );
    }
  }

  async waitIdle(maxMs) {
    const deadline = Date.now() + maxMs;
    for (;;) {
      const state = await this.containerState();
      if (!state.busy) return true;
      if (Date.now() >= deadline) return false;
      await sleep(IDLE_POLL_MS);
    }
  }

  async checkpointWhenIdle(maxMs) {
    const deadline = Date.now() + maxMs;
    for (;;) {
      if (await this.checkpoint()) return true;
      if (Date.now() >= deadline) return false;
      await sleep(IDLE_POLL_MS);
    }
  }

  // Take a snapshot and drop the journal it covers. False while busy.
  async checkpoint() {
    const head = Number(
      this.sql.exec(`SELECT COALESCE(MAX(seq), 0) AS head FROM serve_journal`).one().head,
    );
    const response = await this.upstream(new Request("http://container/celld/snapshot"));
    if (response.status === 409) {
      await response.arrayBuffer();
      return false;
    }
    if (!response.ok) {
      throw new Error(`celld/snapshot answered ${response.status}: ${await response.text()}`);
    }
    const bytes = new Uint8Array(await response.arrayBuffer());
    const next = Number(this.meta("snapshot_seq") ?? 0) + 1;
    this.storage.transactionSync(() => {
      this.sql.exec(`DELETE FROM serve_snapshot`);
      for (let idx = 0, at = 0; at < bytes.length || idx === 0; idx++, at += CHUNK) {
        this.sql.exec(
          `INSERT INTO serve_snapshot (seq, idx, data) VALUES (?, ?, ?)`,
          next,
          idx,
          bytes.slice(at, at + CHUNK).buffer,
        );
      }
      this.sql.exec(`DELETE FROM serve_journal WHERE seq <= ?`, head);
      this.setMeta("snapshot_seq", next);
      this.setMeta("dirty", null);
    });
    return true;
  }
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((n, part) => n + part.byteLength, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.byteLength;
  }
  return out;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
