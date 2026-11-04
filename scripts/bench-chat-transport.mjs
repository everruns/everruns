// Reproducible transport-only fixture, NOT a production Everruns latency test.
// HTTP/1.1 keep-alive + SSE versus uncompressed WS, same command/ack/event JSON.
// --samples=40 --rtts=0,100,300; RTT is injected equally in both directions.
// Requires Node 24+ and apps/ui dependencies. No LLM, TLS, proxy, DB, or server scheduling.
import http from "node:http";
import { once } from "node:events";
import { performance } from "node:perf_hooks";
import { createRequire } from "node:module";
import assert from "node:assert/strict";
import { createEventStream, setEventStreamFactory } from "../apps/ui/src/lib/event-stream.ts";

const require = createRequire(new URL("../apps/ui/package.json", import.meta.url));
const { WebSocket, WebSocketServer } = require("next/dist/compiled/ws");
const samples = Number(
  process.argv.find((arg) => arg.startsWith("--samples="))?.split("=")[1] ?? 40,
);
const rtts = (process.argv.find((arg) => arg.startsWith("--rtts="))?.split("=")[1] ?? "0,100,300")
  .split(",")
  .map(Number);
assert(samples >= 10 && samples <= 1000 && rtts.every((n) => n >= 0 && n <= 1000));
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const percentile = (values, p) =>
  [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * p))];
const metrics = (values) => ({
  p50_ms: +percentile(values, 0.5).toFixed(2),
  p95_ms: +percentile(values, 0.95).toFixed(2),
});

async function run(transport, rtt) {
  let stream;
  const sockets = new Set();
  let received = 0,
    written = 0;
  const server = http.createServer(async (request, response) => {
    if (request.url === "/events") {
      stream = response;
      response.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache" });
      response.write("event: connected\ndata: {}\n\n");
      return;
    }
    let raw = "";
    for await (const chunk of request) raw += chunk;
    await wait(rtt / 2);
    const input = JSON.parse(raw);
    const ack = JSON.stringify({ id: input.id, accepted: true });
    const event = JSON.stringify({
      id: input.id,
      sequence: input.sequence,
      type: "output.message.completed",
      data: { text: "fixture response" },
    });
    await wait(rtt / 2);
    response.writeHead(200, { "Content-Type": "application/json" });
    response.end(ack);
    stream.write(`event: output.message.completed\nid: ${input.sequence}\ndata: ${event}\n\n`);
  });
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.once("close", () => {
      received += socket.bytesRead;
      written += socket.bytesWritten;
      sockets.delete(socket);
    });
  });
  const wss = new WebSocketServer({ server, perMessageDeflate: false });
  wss.on("connection", (socket) =>
    socket.on("message", async (raw) => {
      await wait(rtt / 2);
      const input = JSON.parse(raw.toString());
      await wait(rtt / 2);
      socket.send(JSON.stringify({ id: input.id, accepted: true }));
      socket.send(
        JSON.stringify({
          id: input.id,
          sequence: input.sequence,
          type: "output.message.completed",
          data: { text: "fixture response" },
        }),
      );
    }),
  );
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  const agent = new http.Agent({ keepAlive: true });
  const acknowledgments = new Map(),
    completions = new Map();
  let eventsRequest, ws;
  const observe = (value) => {
    if (value.accepted) acknowledgments.get(value.id)?.();
    else completions.get(value.id)?.();
  };
  if (transport === "sse") {
    await new Promise((resolve) => {
      eventsRequest = http.get(`http://127.0.0.1:${port}/events`, (response) => {
        let buffer = "";
        response.on("data", (chunk) => {
          buffer += chunk;
          let index;
          while ((index = buffer.indexOf("\n\n")) >= 0) {
            const frame = buffer.slice(0, index);
            buffer = buffer.slice(index + 2);
            if (frame.startsWith("event: connected")) resolve();
            else observe(JSON.parse(frame.split("\ndata: ")[1]));
          }
        });
      });
    });
  } else {
    ws = new WebSocket(`ws://127.0.0.1:${port}`, { perMessageDeflate: false });
    await once(ws, "open");
    ws.on("message", (raw) => observe(JSON.parse(raw.toString())));
  }
  const ackTimes = [],
    eventTimes = [];
  for (let index = 0; index < samples + 5; index++) {
    const input = {
      id: `command-${index}`,
      sequence: index,
      message: { role: "user", text: "benchmark command" },
    };
    const raw = JSON.stringify(input);
    const start = performance.now();
    const ack = new Promise((resolve) =>
      acknowledgments.set(input.id, () => resolve(performance.now() - start)),
    );
    const event = new Promise((resolve) =>
      completions.set(input.id, () => resolve(performance.now() - start)),
    );
    if (ws) ws.send(raw);
    else {
      const request = http.request(
        {
          hostname: "127.0.0.1",
          port,
          path: "/commands",
          method: "POST",
          agent,
          headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(raw) },
        },
        (response) => {
          let data = "";
          response.on("data", (chunk) => {
            data += chunk;
          });
          response.on("end", () => observe(JSON.parse(data)));
        },
      );
      request.end(raw);
    }
    const [ackMs, eventMs] = await Promise.all([ack, event]);
    if (index >= 5) {
      ackTimes.push(ackMs);
      eventTimes.push(eventMs);
    }
    acknowledgments.delete(input.id);
    completions.delete(input.id);
  }
  for (const socket of sockets) {
    received += socket.bytesRead;
    written += socket.bytesWritten;
  }
  const bytes = received + written;
  if (ws) {
    ws.close();
    await once(ws, "close");
  }
  eventsRequest?.destroy();
  agent.destroy();
  for (const socket of sockets) socket.destroy();
  await new Promise((resolve) => wss.close(resolve));
  await new Promise((resolve) => server.close(resolve));
  return {
    transport,
    injected_rtt_ms: rtt,
    samples,
    ack: metrics(ackTimes),
    event: metrics(eventTimes),
    wire_bytes_including_setup_and_warmup: bytes,
  };
}

// Exercise the actual UI sharing implementation, not a duplicate model of it.
globalThis.window = { location: { origin: "http://benchmark.local" } };
let opened = 0,
  live = 0;
setEventStreamFactory(() => {
  opened++;
  live++;
  return {
    addEventListener() {},
    close() {
      live--;
    },
    onopen: null,
    onerror: null,
  };
});
// Unscoped factory calls reproduce the pre-sharing control path.
const control = [];
for (let index = 0; index < 20; index++) {
  const url = `/api/v1/sessions/session_${index}/sse`;
  control.push(createEventStream(url + "?after_sequence=0"));
  control.push(createEventStream(url));
}
const previousConnections = live;
assert.equal(previousConnections, 40);
for (const consumer of control) consumer.close();
assert.equal(live, 0);
opened = 0;
const consumers = [];
for (let index = 0; index < 20; index++) {
  const url = `/api/v1/sessions/session_${index}/sse`;
  consumers.push(createEventStream(url + "?after_sequence=0", { shareScope: "benchmark" }));
  consumers.push(createEventStream(url, { shareScope: "benchmark", snapshotOnly: true }));
}
assert.equal(live, 20);
const sharing = {
  sessions: 20,
  consumers: 40,
  previous_connections: previousConnections,
  shared_connections: live,
  opened,
};
for (const consumer of consumers) consumer.close();
assert.equal(live, 0);
opened = 0;
const promoted = [];
for (let index = 0; index < 20; index++) {
  const url = `/api/v1/sessions/session_${index}/sse`;
  promoted.push(createEventStream(url, { shareScope: "benchmark", snapshotOnly: true }));
  promoted.push(createEventStream(url + "?after_sequence=0", { shareScope: "benchmark" }));
}
assert.equal(live, 20);
sharing.task_first_connections = live;
sharing.task_first_opened_including_promotion = opened;
for (const consumer of promoted) consumer.close();
assert.equal(live, 0);
setEventStreamFactory(null);
const results = [];
for (const rtt of rtts)
  for (const transport of ["sse", "websocket"]) {
    const result = await run(transport, rtt);
    results.push(result);
    console.error(JSON.stringify(result));
  }
console.log(
  JSON.stringify(
    {
      node: process.version,
      fixture: "loopback HTTP/1.1 keep-alive, no TLS/proxy/LLM, compression disabled",
      sharing,
      results,
    },
    null,
    2,
  ),
);
