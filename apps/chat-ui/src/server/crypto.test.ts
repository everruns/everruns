import { test } from "node:test";
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { open, seal } from "./crypto";
test("encrypted credentials are authenticated and bound to the cookie hash", () => {
  const key = randomBytes(32).toString("hex"),
    payload = { channel: "one", identity: "test-provider-token" };
  const ciphertext = seal(payload, key, "cookie-hash-a");
  assert.deepEqual(open(ciphertext, key, "cookie-hash-a"), payload);
  assert.throws(() => open(ciphertext, key, "cookie-hash-b"));
  assert.throws(() => open(ciphertext, randomBytes(32).toString("hex"), "cookie-hash-a"));
  const changed = Buffer.from(ciphertext, "base64");
  changed[changed.length - 1] ^= 1;
  assert.throws(() => open(changed.toString("base64"), key, "cookie-hash-a"));
  assert.notEqual(
    seal(payload, key, "cookie-hash-a"),
    ciphertext,
    "Each encryption needs a new IV",
  );
  assert.throws(() => seal(payload, "00", "cookie-hash-a"));
});

test("credential classes cannot be substituted at the same opaque hash", () => {
  const key = "34".repeat(32);
  const encrypted = seal({ identity: "provider-token" }, key, "handoff:cookie-hash");
  assert.throws(() => open(encrypted, key, "session:cookie-hash"));
});
