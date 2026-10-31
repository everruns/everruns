import { test } from "node:test";
import assert from "node:assert/strict";
import { liveStream, proxyHeaders } from "./proxy";

test("a live session stream is not treated as a short request", () => {
  assert.equal(liveStream("sessions/session_1/sse"), true);
  assert.equal(liveStream("sessions/session_1"), false);
  assert.equal(liveStream("sessions/session_1/events"), false);
  const headers = proxyHeaders("sessions/session_1/sse", "text/event-stream");
  assert.equal(headers["Content-Type"], "text/event-stream");
  assert.equal(headers["Cache-Control"], "no-cache, no-transform");
  assert.equal(headers["X-Accel-Buffering"], "no");
});

test("ordinary proxy responses stay uncached JSON", () => {
  const headers = proxyHeaders("sessions/session_1", null);
  assert.equal(headers["Content-Type"], "application/json");
  assert.equal(headers["Cache-Control"], "no-store");
});
