// A serve app on celld. Each cell (a Durable Object, addressed by name) runs
// one serve container and keeps its state durable; see `cell.js`.
//
//   /cells/<name>/v1/...   serve's wire API on the cell named <name>
//   /cells/<name>/_cell/state   what the cell holds (snapshot, journal)
//
// Decision: the cell is the unit of durability and of placement. Name one
// per conversation (an AG-UI thread) or per user; everything under a name
// shares one container, and a long turn there delays session creation in
// the same cell until it can snapshot.
import { Container } from "@cloudflare/containers";
import { DurableObject } from "cloudflare:workers";
import { Cell } from "./cell.js";

// Variables passed into the container: serve's own settings and the
// app's declared secrets. List extra names in CONTAINER_ENV.
function containerEnv(env) {
  const extra = (env.CONTAINER_ENV ?? "").split(",").map((name) => name.trim());
  const out = {};
  for (const [name, value] of Object.entries(env)) {
    if (typeof value !== "string") continue;
    if (name.startsWith("SERVE_") || extra.includes(name)) out[name] = value;
  }
  return out;
}

/** The production cell: supervises a container from the `containers` entry. */
export class ServeCell extends Container {
  defaultPort = 8080;
  sleepAfter = "10m";

  constructor(ctx, env) {
    super(ctx, env);
    this.envVars = containerEnv(env);
    this.cell = new Cell({
      storage: ctx.storage,
      upstream: (request) => this.containerFetch(request),
      // The Container class owns the alarm; its scheduler calls `flush`.
      later: (seconds) => this.schedule(seconds, "flush"),
    });
  }

  fetch(request) {
    return this.cell.handle(request);
  }

  flush() {
    return this.cell.flush();
  }
}

/**
 * The same cell over a serve process reached by URL (SERVE_URL) instead of
 * a container: for trying the protocol on a machine without a container
 * engine, and for the end-to-end test.
 */
export class ExternalServeCell extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    const base = env.SERVE_URL;
    this.cell = new Cell({
      storage: ctx.storage,
      upstream: (request) => {
        const url = new URL(request.url);
        return fetch(new Request(new URL(url.pathname + url.search, base), request));
      },
      later: async (seconds) => {
        const at = Date.now() + seconds * 1000;
        const current = await ctx.storage.getAlarm();
        if (current === null || current > at) await ctx.storage.setAlarm(at);
      },
    });
  }

  fetch(request) {
    return this.cell.handle(request);
  }

  alarm() {
    return this.cell.flush();
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const match = url.pathname.match(/^\/cells\/([A-Za-z0-9_.-]{1,128})(\/.*)$/);
    if (!match) {
      return new Response("Try /cells/<name>/v1/agent\n", {
        status: 404,
        headers: { "content-type": "text/plain" },
      });
    }
    const [, name, rest] = match;
    const cell = env.CELL.get(env.CELL.idFromName(name));
    return cell.fetch(new Request(new URL(rest + url.search, url.origin), request));
  },
};
