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
    const { pathname } = new URL(req.url);
    // Like html_handling "auto-trailing-slash": a page asked without its slash.
    if (this.files[`${pathname}/`]) {
      return new Response(null, { status: 307, headers: { location: `${pathname}/` } });
    }
    const file = this.files[pathname];
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

test("the former docs host redirects every URL to the same page under /docs", async () => {
  for (const [from, to] of [
    ["https://docs.everruns.com/", "https://everruns.com/docs/"],
    ["https://docs.everruns.com/framework/quickstart/?ref=x", "https://everruns.com/docs/framework/quickstart/?ref=x"],
    ["https://docs.everruns.com/llms.txt", "https://everruns.com/docs/llms.txt"],
  ]) {
    const response = await worker.fetch(new Request(from, { headers: { accept: "text/markdown" } }), { ASSETS });
    assert.equal(response.status, 301);
    assert.equal(response.headers.get("location"), to);
  }
});

test("a page asked without its trailing slash redirects permanently, query kept", async () => {
  const response = await worker.fetch(request("/docs/framework?ref=x"), { ASSETS });
  assert.equal(response.status, 301);
  assert.equal(response.headers.get("location"), "https://everruns.com/docs/framework/?ref=x");
});


test("shared cost-estimator announcement redirects to smart projections for every reader", async () => {
  for (const suffix of ["", "/", ".md", "/index.md"]) {
    for (const headers of [{}, {accept:"text/markdown"}, {"user-agent":"Googlebot"}]) {
      const response = await worker.fetch(request(`/docs/changelog/2026-10-28-agent-cost-estimation${suffix}?ref=social`, headers), {ASSETS});
      assert.equal(response.status, 301);
      assert.equal(response.headers.get("location"), `https://everruns.com/docs/changelog/2026-10-28-smart-agent-cost-projections${suffix.endsWith(".md") ? "/index.md" : "/"}?ref=social`);
    }
  }
});


test("legacy gallery links redirect to Reference for HTML and Markdown", async () => {
  for (const suffix of ["", "/", ".md", "/index.md"]) {
    const response = await worker.fetch(request(`/docs/features/generative-ui-gallery${suffix}?from=guide`), { ASSETS });
    assert.equal(response.status, 301);
    assert.equal(response.headers.get("location"), `https://everruns.com/docs/reference/generative-ui${suffix.endsWith(".md") ? "/index.md" : "/"}?from=guide`);
  }
});
