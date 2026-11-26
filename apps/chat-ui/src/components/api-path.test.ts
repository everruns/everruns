import { test } from "node:test";
import assert from "node:assert/strict";
import { apiPath } from "./api-path";
test("chat tabs select distinct bounded API paths while accounts are Application-wide", () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
  try {
    for (const [pathname, expected] of [
      ["/chats/writer", "/api/chats/writer/sessions"],
      ["/chats/reviewer", "/api/chats/reviewer/sessions"],
      ["/", "/api/sessions"],
      ["/chats/../../platform", "/api/sessions"],
    ]) {
      Object.defineProperty(globalThis, "window", {
        value: { location: { pathname } },
        configurable: true,
      });
      assert.equal(apiPath("sessions"), expected);
      assert.equal(apiPath("chat/profile"), "/api/chat/profile");
      assert.equal(apiPath("virtual-users/me"), "/api/virtual-users/me");
    }
  } finally {
    if (previous) Object.defineProperty(globalThis, "window", previous);
    else Reflect.deleteProperty(globalThis, "window");
  }
});
