import { test } from "node:test";
import assert from "node:assert/strict";
import { presentViewer, viewerFromIdToken } from "../profile";

function token(claims: unknown) {
  return `e30.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.sig`;
}

test("a verified identity projects a name, email and https picture", () => {
  assert.deepEqual(
    presentViewer({
      sub: "secret-subject",
      name: "Ada Lovelace",
      email: "ada@example.com",
      email_verified: true,
      picture: "https://cdn.example/ada.png",
      preferred_username: "ada",
    }),
    {
      name: "Ada Lovelace",
      email: "ada@example.com",
      picture: "https://cdn.example/ada.png",
    },
  );
});

test("name falls back through the profile claims and keeps a unicode name", () => {
  assert.equal(presentViewer({ given_name: "Ada", family_name: "Lovelace" })?.name, "Ada Lovelace");
  assert.equal(presentViewer({ preferred_username: "ada" })?.name, "ada");
  assert.equal(presentViewer({ email: "ada@example.com" })?.name, "ada@example.com");
  assert.equal(presentViewer({ email: "ada@example.com" })?.email, undefined);
  const encoded = viewerFromIdToken(token({ name: "Ada Løvelace", email: "ada@example.com" }));
  assert.equal(encoded?.name, "Ada Løvelace");
  assert.equal(encoded?.email, "ada@example.com");
});

test("an unverified or unsafe profile is not shown", () => {
  assert.equal(presentViewer({ email: "ada@example.com", email_verified: false }), null);
  assert.equal(presentViewer({ name: "Ada", email: "ada@example.com", email_verified: false })?.email, undefined);
  assert.equal(presentViewer({ name: "Ada", picture: "http://cdn.example/ada.png" })?.picture, undefined);
  assert.equal(presentViewer({ name: "Ada", picture: "javascript:alert(1)" })?.picture, undefined);
  assert.equal(
    presentViewer({ name: "Ada", picture: "https://user:pass@cdn.example/ada.png" })?.picture,
    undefined,
  );
  assert.equal(presentViewer({ name: "Ada\u202elove" }), null);
  assert.equal(presentViewer({ name: "a".repeat(121) }), null);
  assert.equal(presentViewer({ sub: "only-a-subject" }), null);
  assert.equal(presentViewer(null), null);
  assert.equal(viewerFromIdToken("not-a-token"), null);
});
