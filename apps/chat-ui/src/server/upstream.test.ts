import assert from "node:assert/strict";
import { createServer } from "node:http";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { fetchUpstream } from "./upstream";

test("SSE outlives the header deadline and still follows browser cancellation", async (t) => {
  const server = createServer((_request, response) => {
    response.writeHead(200, { "Content-Type": "text/event-stream" });
    response.write(": connected\n\n");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => {
    server.closeAllConnections();
    server.close();
  });
  const address = server.address() as { port: number };
  const browser = new AbortController();
  const response = await fetchUpstream(
    `http://127.0.0.1:${address.port}`,
    {
      signal: browser.signal,
    },
    true,
    200,
  );
  const reader = response.body!.getReader();
  assert.equal((await reader.read()).done, false);
  await delay(350);
  const next = reader.read();
  const outcome = await Promise.race([
    next.then(
      () => "closed",
      () => "aborted",
    ),
    delay(30).then(() => "open"),
  ]);
  assert.equal(outcome, "open", "the request deadline must not abort an established SSE body");
  browser.abort();
  await assert.rejects(next);
});

for (const stream of [true, false]) {
  test(`upstream headers stay bounded (stream=${stream})`, async (t) => {
    const server = createServer(() => {});
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    t.after(() => {
      server.closeAllConnections();
      server.close();
    });
    const address = server.address() as { port: number };
    await assert.rejects(fetchUpstream(`http://127.0.0.1:${address.port}`, {}, stream, 50));
  });
  test(`ordinary bodies keep their deadline (stream=${stream})`, async (t) => {
    const server = createServer((_request, response) => {
      response.writeHead(200, { "Content-Type": "application/json" });
      response.write('{"pending":');
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    t.after(() => {
      server.closeAllConnections();
      server.close();
    });
    const address = server.address() as { port: number };
    const response = await fetchUpstream(`http://127.0.0.1:${address.port}`, {}, stream, 200);
    await assert.rejects(response.text());
  });
}
