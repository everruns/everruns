import assert from "node:assert/strict";
import { test } from "node:test";
import worker, { markdownPath, wantsMarkdown } from "./index.mjs";

const request = (path, headers = {}) => new Request(`https://everruns.com${path}`, { headers });

// A stand-in for the static assets binding: two pages and their Markdown twins.
const ASSETS = {
  files: {
    "/docs/": ["<h1>Docs</h1>", "text/html"],
    "/docs/index.md": ["# Docs", "text/markdown"],
    "/docs/framework/": ["<h1>Framework</h1>", "text/html"],
    "/docs/framework/index.md": ["# Framework", "text/markdown"],
  },
  async fetch(req) {
    const file = this.files[new URL(req.url).pathname];
    return file
      ? new Response(file[0], { headers: { "Content-Type": file[1] } })
      : new Response("not found", { status: 404 });
  },
};

test("markdown twin path only for page URLs under /docs", () => {
  assert.equal(markdownPath("/docs/"), "/docs/index.md");
  assert.equal(markdownPath("/docs/framework/"), "/docs/framework/index.md");
  assert.equal(markdownPath("/docs/llms.txt"), null);
  assert.equal(markdownPath("/pricing/"), null);
});

test("AI clients want Markdown, crawlers never do", () => {
  assert.equal(wantsMarkdown(request("/docs/", { accept: "text/markdown" })), true);
  assert.equal(wantsMarkdown(request("/docs/", { "user-agent": "ClaudeBot/1.0" })), true);
  assert.equal(wantsMarkdown(request("/docs/", { accept: "text/html" })), false);
  assert.equal(
    wantsMarkdown(request("/docs/", { accept: "text/markdown", "user-agent": "Googlebot/2.1" })),
    false,
  );
});

test("Accept: text/markdown on a page URL returns the Markdown twin", async () => {
  const response = await worker.fetch(request("/docs/framework/", { accept: "text/markdown" }), { ASSETS });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/markdown; charset=utf-8");
  assert.equal(await response.text(), "# Framework");
  assert.match(response.headers.get("vary"), /Accept/);
});

test("browsers get HTML, with discovery links", async () => {
  const response = await worker.fetch(request("/docs/framework/", { accept: "text/html" }), { ASSETS });
  assert.equal(await response.text(), "<h1>Framework</h1>");
  assert.match(response.headers.get("link"), /llms\.txt/);
});

test("a page without a twin falls back to the asset response", async () => {
  const response = await worker.fetch(request("/docs/missing/", { accept: "text/markdown" }), { ASSETS });
  assert.equal(response.status, 404);
});

test("the bare root redirects into the docs", async () => {
  for (const path of ["/", "/docs"]) {
    const response = await worker.fetch(request(path), { ASSETS });
    assert.equal(response.status, 301);
    assert.equal(response.headers.get("location"), "https://everruns.com/docs/");
  }
});
