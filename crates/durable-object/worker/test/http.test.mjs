import test from "node:test";
import assert from "node:assert/strict";
import { address } from "../src/http.js";

const tenant = "test";
const sessionId = `session_${"1".repeat(32)}`;
const request = (operation, owner = tenant) =>
  new Request(`https://authority.test/tenants/${owner}/sessions/${sessionId}/${operation}`);

test("authority routing accepts underscore operations used by Engine commands and cursors", () => {
  for (const operation of ["resolve_waits", "event_sequence", "status"]) {
    assert.deepEqual(address(request(operation), { TENANT_ID: tenant }), {
      tenant, sessionId, operation,
    });
  }
});

test("authority routing keeps tenant confinement and bounded single-segment operations", () => {
  for (const operation of ["resolve/waits", "resolve%2Fwaits", "Resolve_waits", "a".repeat(65)]) {
    assert.throws(() => address(request(operation), { TENANT_ID: tenant }), /unknown authority address/);
  }
  assert.throws(() => address(request("status", "other"), { TENANT_ID: tenant }), /unknown authority address/);
});
