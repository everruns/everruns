import { test } from "node:test";
import assert from "node:assert/strict";
import { hostLabel, chatLogo, safeMutation, hostOf } from "./config";
test("one DNS label selects one chat; a channel id, the broker and lookalikes do not", () => {
  const label = "k3m9pq2r7x";
  const id = "01933b5a000070008000000000000001";
  assert.equal(hostLabel(`${label}.apps.example.com`, "apps.example.com"), label);
  assert.equal(hostLabel(`${label.toUpperCase()}.apps.example.com`, "apps.example.com"), label);
  for (const host of [
    `${id}.apps.example.com`,
    "login.apps.example.com",
    `${label}.evilapps.example.com`,
    `${label}.apps.example.com.evil.test`,
    `evil.${label}.apps.example.com`,
    "apps.example.com",
    `appchan_${id}.apps.example.com`,
  ]) {
    assert.equal(hostLabel(host, "apps.example.com"), null, host);
  }
});
test("cookie requests require the exact origin, including a sibling channel", () => {
  const url = "https://k3m9pq2r7x.apps.example.com/api/sessions";
  assert.equal(
    safeMutation(new Request(url, { headers: { origin: "https://k3m9pq2r7x.apps.example.com" } })),
    true,
  );
  for (const origin of [
    "https://n4w8st6y2v.apps.example.com",
    "https://apps.example.com",
    "null",
    "http://k3m9pq2r7x.apps.example.com",
  ]) {
    assert.equal(safeMutation(new Request(url, { headers: { origin } })), false);
  }
  assert.equal(safeMutation(new Request(url)), false);
});

test("only this channel's circle avatar becomes a same-origin logo", () => {
  const id = "01933b5a000070008000000000000001";
  assert.equal(
    chatLogo(`/v1/avatars/avatar_${id}/circle-64.png`),
    `/api/chat/logo/avatar_${id}`,
  );
  for (const value of [
    null,
    `/v1/avatars/avatar_${id}/square-64.png`,
    `/v1/avatars/avatar_${id}/circle-256.png`,
    "https://cdn.example/logo.png",
    `/v1/avatars/avatar_${id}/circle-64.png/../source.png`,
  ]) {
    assert.equal(chatLogo(value), undefined);
  }
});

test("preserved Host selects the channel behind a proxy listener", () => {
  const host = "k3m9pq2r7x.example.test";
  const request = new Request("http://localhost:3000/api/sessions", {
    headers: { host, "x-forwarded-host": "attacker.example.test", origin: `https://${host}` },
  });
  assert.equal(hostOf(request), host);
  assert.equal(safeMutation(request), true);
});
