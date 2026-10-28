import test from "node:test";
import assert from "node:assert/strict";
import { consumeSse } from "../client/sse";
import {
  recoveryKey,
  readRecovery,
  saveRecovery,
  saveDraft,
  clearRecovery,
  retryable,
} from "../client/recovery";

test("SSE preserves UTF-8, CRLF and multiline data at every byte boundary", async () => {
  const bytes = new TextEncoder().encode(
    ': heartbeat\r\nevent: output.message.completed\r\ndata: {"text":\r\ndata: "Вітаю 🌍"}\r\n\r\n',
  );
  for (let split = 0; split <= bytes.length; split++) {
    const seen: string[] = [];
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(bytes.slice(0, split));
        controller.enqueue(bytes.slice(split));
        controller.close();
      },
    });
    await consumeSse(stream, (type, data) => {
      seen.push(type);
      assert.deepEqual(JSON.parse(data), { text: "Вітаю 🌍" });
    });
    assert.deepEqual(seen, ["output.message.completed"], `split ${split}`);
  }
});
test("SSE rejects an unbounded partial frame and cancels its reader", async () => {
  let cancelled = false;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new Uint8Array(8 * 1024 * 1024 + 1).fill(65));
    },
    cancel() {
      cancelled = true;
    },
  });
  await assert.rejects(
    consumeSse(stream, () => assert.fail()),
    /too large/,
  );
  assert.equal(cancelled, true);
});
test("consumer recovery keeps the original request key, separates login scopes, and retires on logout", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
      Object.defineProperty(storage, key, { value, configurable: true, enumerable: true });
    },
    removeItem: (key: string) => {
      values.delete(key);
      delete (storage as Record<string, unknown>)[key];
    },
  };
  Object.defineProperty(globalThis, "sessionStorage", { value: storage, configurable: true });
  const key = recoveryKey("alice-login", "session_1"),
    pending = { key: "original-id", body: { message: "hello" }, createdAt: Date.now() };
  saveRecovery(key, { draft: "hello", pending });
  const bodyRecord = storage.getItem(key);
  saveDraft(key, "newer draft");
  assert.equal(storage.getItem(key), bodyRecord, "keystrokes do not rewrite the request payload");
  assert.equal(readRecovery(key).draft, "newer draft");
  saveDraft(key, "hello");
  assert.deepEqual(readRecovery(key), { draft: "hello", pending });
  assert.deepEqual(readRecovery(recoveryKey("bob-login", "session_1")), { draft: "" });
  assert.equal(retryable(pending), true);
  assert.equal(retryable({ ...pending, createdAt: Date.now() - 24 * 60 * 60 * 1000 }), false);
  storage.setItem("theme", "light");
  clearRecovery();
  assert.deepEqual(readRecovery(key), { draft: "" });
  assert.equal(storage.getItem("theme"), "light");
});
