// Additional Everruns regression; this is not an upstream conformance case.
import assert from "node:assert/strict";
import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { createServer } from "node:http";

export async function checkDeliveryIdentities(base, portFile) {
  const key = randomBytes(32);
  const secret = `whsec_${key.toString("base64")}`;
  const deliveries = [];
  const subscriptions = [];
  let verifications = 0;
  const receiver = createServer(async (request, response) => {
    const chunks = [];
    let size = 0;
    for await (const chunk of request) {
      size += chunk.length;
      if (size > 256 * 1024) return response.writeHead(413).end();
      chunks.push(chunk);
    }
    const raw = Buffer.concat(chunks);
    const id = request.headers["webhook-id"];
    const timestamp = request.headers["webhook-timestamp"];
    const signature = createHmac("sha256", key)
      .update(`${id}.${timestamp}.`)
      .update(raw)
      .digest("base64");
    const expected = Buffer.from(`v1,${signature}`);
    const actual = Buffer.from(request.headers["webhook-signature"] ?? "");
    if (actual.length !== expected.length || !timingSafeEqual(actual, expected))
      return response.writeHead(401).end();
    const body = JSON.parse(raw);
    if (body.type === "verification") {
      verifications += 1;
      return response
        .writeHead(200, { "content-type": "application/json" })
        .end(JSON.stringify({ challenge: body.challenge }));
    }
    deliveries.push({
      event: body.eventId,
      subscription: request.headers["x-mcp-subscription-id"],
      id,
      session: body.data?.session_id,
    });
    response.writeHead(200).end();
  });
  await new Promise((resolve) => receiver.listen(0, "127.0.0.1", resolve));
  await writeFile(portFile, String(receiver.address().port));
  async function rpc(method, params) {
    const response = await fetch(`${base}/mcp`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "MCP-Protocol-Version": "2026-07-28",
        "Mcp-Method": method,
        "Mcp-Name": "session.completed",
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
      signal: AbortSignal.timeout(5000),
    });
    assert.equal(response.status, 200);
    const result = await response.json();
    assert.equal(result.error, undefined, JSON.stringify(result.error));
    return result.result;
  }
  try {
    for (const suffix of ["a", "b"]) {
      const params = {
        name: "session.completed",
        arguments: {},
        delivery: {
          mode: "webhook",
          url: `https://8.8.8.8/message-identity-${suffix}`,
          secret,
        },
      };
      const result = await rpc("events/subscribe", params);
      subscriptions.push({ params, id: result.id });
    }
    const deadline = Date.now() + 5000;
    while (Date.now() < deadline) {
      const events = new Map();
      for (const delivery of deliveries) {
        const rows = events.get(delivery.event) ?? [];
        rows.push(delivery);
        events.set(delivery.event, rows);
      }
      for (const rows of events.values()) {
        if (new Set(rows.map((row) => row.subscription)).size !== 2) continue;
        assert.equal(
          new Set(rows.map((row) => row.id)).size,
          2,
          "one event on two subscriptions needs independent webhook-id values",
        );
        for (const row of rows) assert.match(row.id, /^msg_[A-Za-z0-9_-]+$/);
        const refreshed = await rpc(
          "events/subscribe",
          subscriptions[0].params,
        );
        assert.equal(refreshed.id, subscriptions[0].id);
        const filtered = {
          ...subscriptions[0].params,
          arguments: { session_id: rows[0].session },
        };
        const added = await rpc("events/subscribe", filtered);
        subscriptions.push({ params: filtered, id: added.id });
        assert.notEqual(added.id, refreshed.id);
        assert.equal(
          verifications,
          2,
          "refresh and arguments reuse principal/URL consent",
        );
        return [
          {
            id: "everruns-webhook-message-identity",
            source: "everruns",
            status: "SUCCESS",
          },
          {
            id: "everruns-webhook-consent-cache",
            source: "everruns",
            status: "SUCCESS",
          },
        ];
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    assert.fail("no event was observed on both subscriptions");
  } finally {
    try {
      for (const { params } of subscriptions)
        await rpc("events/unsubscribe", params);
    } finally {
      await new Promise((resolve) => receiver.close(resolve));
    }
  }
}
