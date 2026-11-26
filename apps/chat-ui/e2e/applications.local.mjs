// Invoked by scripts/test-applications-local.py against the real Rust API.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { chromium, expect } from "@playwright/test";
const cfg = JSON.parse(readFileSync(process.argv[2], "utf8"));
const browser = await chromium.launch({ headless: true });
try {
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    viewport: { width: 1440, height: 1000 },
    deviceScaleFactor: 2,
  });
  const [name, value] = cfg.cookie.split("=");
  await context.addCookies([
    {
      name,
      value,
      domain: new URL(cfg.origin).hostname,
      path: "/",
      secure: true,
      httpOnly: true,
      sameSite: "Lax",
    },
  ]);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(cfg.origin);
  await page.getByRole("heading", { name: "Content Studio", exact: true }).waitFor();
  await page.screenshot({ path: cfg.results + "/application-overview@2x.png" });
  async function call(path, method = "GET", data) {
    const r = await context.request.fetch(cfg.origin + "/api/" + path, {
      method,
      data,
      headers: { Origin: cfg.origin },
    });
    assert.ok(r.ok(), `${method} ${path}: ${r.status()} ${await r.text()}`);
    return r.status() === 204 ? null : r.json();
  }
  // The selected Chat is in the URL, so independent tabs cannot switch peers.
  const a = await call("chats/writer/sessions", "POST", { title: "Editorial draft" });
  const b = await call("chats/reviewer/sessions", "POST", { title: "Editorial review" });
  assert.notEqual(a.id, b.id);
  assert.equal(
    (await context.request.get(cfg.origin + "/api/chats/writer/sessions/" + b.id)).status(),
    404,
  );
  await page.goto(cfg.origin + "/chats/writer");
  await page.getByRole("textbox", { name: "Message", exact: true }).waitFor();
  await expect(page.getByRole("navigation", { name: "Conversations", exact: true })).toContainText(
    "Editorial draft",
  );
  const reviewTab = await context.newPage();
  await reviewTab.goto(cfg.origin + "/chats/reviewer");
  await reviewTab.getByRole("textbox", { name: "Message", exact: true }).waitFor();
  await expect(
    reviewTab.getByRole("navigation", { name: "Conversations", exact: true }),
  ).toContainText("Editorial review");
  await expect(
    page.getByRole("navigation", { name: "Conversations", exact: true }),
  ).not.toContainText("Editorial review");
  await reviewTab.close();
  await page.goto(cfg.origin);
  await page.getByRole("link", { name: /Editorial room Work with/ }).click();
  await page.getByRole("textbox", { name: "Message the room" }).fill("Draft review starts here.");
  await page.getByRole("button", { name: "Send message", exact: true }).click();
  await page.getByText("Draft review starts here.", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Reply", exact: true }).click();
  await page.getByRole("textbox", { name: "Reply to thread" }).fill("The outline is ready.");
  await page.getByRole("button", { name: "Send message", exact: true }).click();
  await page.getByText("The outline is ready.", { exact: true }).waitFor();
  await page.screenshot({ path: cfg.results + "/application-room@2x.png" });
  const messages = await call("application/rooms/editorial/messages");
  assert.equal(messages.data[0].text, "Draft review starts here.");
  assert.ok(!("session_id" in messages.data[0]));
  await call("application/memory/handbook", "PUT", {
    path: "/voice.md",
    content: "# Our voice\n\nWrite clearly. Use examples. Respect the reader.",
    create: true,
  });
  assert.match(
    (await call("application/memory/handbook?file=true&path=%2Fvoice.md")).content,
    /Write clearly/,
  );
  await page.goto(cfg.origin + "/memory/handbook");
  await page.getByRole("button", { name: /voice.md/ }).click();
  const editor = page.getByRole("textbox", { name: "File content" });
  await expect(editor).toHaveValue(/Write clearly/);
  await editor.fill(
    "# Our voice\n\nWrite clearly. Use examples. Respect the reader.\n\nReview together before publishing.",
  );
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page.getByText("Saved", { exact: true }).waitFor();
  await page.reload();
  await page.getByRole("button", { name: /voice.md/ }).click();
  await expect(editor).toHaveValue(/Review together/);
  await page.screenshot({ path: cfg.results + "/application-memory@2x.png" });
  const denied = await context.request.put(cfg.origin + "/api/application/memory/handbook", {
    headers: { Origin: "https://sibling.capacity.localhost" },
    data: { path: "/voice.md", content: "cross-origin" },
  });
  assert.equal(denied.status(), 403);
  assert.match(
    (await call("application/memory/handbook?file=true&path=%2Fvoice.md")).content,
    /Write clearly/,
  );
  assert.deepEqual(errors, []);
  console.log(
    "PASS: browser overview, Room attribution/replies, shared memory, multi-chat isolation, sibling-origin denial",
  );
} finally {
  await browser.close();
}
