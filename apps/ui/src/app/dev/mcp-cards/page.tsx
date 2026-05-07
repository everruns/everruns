"use client";

// Dev showcase for the MCP Apps card iframe renderer.
// See specs/mcp-cards.md for the card standard.

import Link from "next/link";
import { useState } from "react";
import { ArrowLeft } from "lucide-react";
import { McpCardIframe, type CardMessage } from "@/components/mcp/mcp-card-iframe";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { usePageTitle } from "@/hooks";

const isDev = process.env.NODE_ENV === "development";

const SAMPLE_AGENT_HTML = `<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; connect-src 'none'">
<meta name="viewport" content="width=360">
<title>Everruns: Customer Support</title>
<style>
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body {
  margin: 0;
  font: 13px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  background: transparent;
  color: #111;
}
@media (prefers-color-scheme: dark) { body { color: #eee; } }
.card {
  width: 360px;
  max-width: 100%;
  padding: 16px;
  border-radius: 12px;
  border: 1px solid rgba(127,127,127,0.25);
  background: rgba(255,255,255,0.6);
}
@media (prefers-color-scheme: dark) {
  .card { background: rgba(20,20,22,0.6); }
}
.card-header { display: grid; grid-template-columns: auto 1fr; gap: 4px 10px; align-items: baseline; }
.card-kind { grid-column: 1 / -1; font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; opacity: 0.6; }
.card-title { grid-column: 1 / -1; font-size: 18px; font-weight: 600; margin: 0; }
.card-subtitle { grid-column: 1 / -1; font-size: 12px; opacity: 0.7; }
.card-id { grid-column: 1 / -1; font-size: 10px; opacity: 0.55; word-break: break-all; }
.card-status {
  grid-column: 1 / -1; justify-self: start;
  font-size: 11px; padding: 2px 8px; border-radius: 999px;
  border: 1px solid currentColor; opacity: 0.85;
}
.status-active { color: #1a7f37; }
.status-archived { color: #9a6700; }
.status-deleted { color: #cf222e; }
.card-description { margin: 12px 0 0; font-size: 13px; opacity: 0.9; white-space: pre-wrap; }
.card-tags { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
.tag { font-size: 11px; padding: 1px 6px; border-radius: 4px; background: rgba(127,127,127,0.18); }
.card-stats { display: grid; grid-template-columns: 1fr 1fr; gap: 8px 16px; margin: 14px 0 0; padding: 0; }
.stat { display: contents; }
.stat dt { font-size: 11px; opacity: 0.6; text-transform: uppercase; letter-spacing: 0.04em; }
.stat dd { margin: 0; font-variant-numeric: tabular-nums; font-weight: 500; }
.card-footer { margin-top: 12px; font-size: 11px; opacity: 0.6; display: grid; gap: 2px; }
.card-actions { margin-top: 12px; display: flex; gap: 6px; flex-wrap: wrap; }
.card-actions:empty { display: none; }
button.card-action {
  font: inherit;
  padding: 4px 10px;
  border-radius: 6px;
  border: 1px solid rgba(127,127,127,0.4);
  background: transparent;
  color: inherit;
  cursor: pointer;
}
button.card-action:hover { background: rgba(127,127,127,0.1); }
</style>
</head><body><article class="card">
<header class="card-header">
  <div class="card-kind">Agent</div>
  <h1 class="card-title">Customer Support</h1>
  <div class="card-subtitle">customer-support</div>
  <div class="card-status status-active">active</div>
  <code class="card-id">agent_01933b5a000070008000000000000001</code>
</header>
<p class="card-description">Handles tier-1 questions about billing and account access.</p>
<div class="card-tags">
  <span class="tag">faq</span>
  <span class="tag">tier-1</span>
</div>
<dl class="card-stats">
  <div class="stat"><dt>Input tokens</dt><dd>1,234,567</dd></div>
  <div class="stat"><dt>Output tokens</dt><dd>89,012</dd></div>
  <div class="stat"><dt>Cache read</dt><dd>50,000</dd></div>
  <div class="stat"><dt>Sessions</dt><dd>42</dd></div>
</dl>
<footer class="card-footer">
  <div>Created Jan 02, 2026</div>
  <div>Updated Jan 05, 2026</div>
</footer>
<div class="card-actions" data-card-actions>
  <button class="card-action" id="run">Run</button>
  <button class="card-action" id="open">Open</button>
</div>
<script>
// Phase-2 preview: action buttons post structured messages back to host.
document.getElementById("run").addEventListener("click", function () {
  parent.postMessage({
    type: "tool",
    payload: { toolName: "agent_run", params: { agent_id: "agent_01933b5a000070008000000000000001", message: "Hello" } }
  }, "*");
});
document.getElementById("open").addEventListener("click", function () {
  parent.postMessage({
    type: "intent",
    payload: { intent: "open_agent", params: { agent_id: "agent_01933b5a000070008000000000000001" } }
  }, "*");
});
</script>
</article></body></html>`;

export default function McpCardsDevPage() {
  usePageTitle(isDev ? "Dev: MCP Cards" : "Page not found");

  const [log, setLog] = useState<CardMessage[]>([]);

  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="text-muted-foreground mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background px-4 py-8">
      <div className="mx-auto max-w-4xl space-y-6">
        <Link
          href="/dev"
          className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" /> Back to dev
        </Link>

        <div>
          <h1 className="text-2xl font-semibold tracking-tight">MCP Cards</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Sandboxed iframe renderer for <code>ui://everruns/...</code> embedded resources returned
            by the Everruns MCP endpoint. The HTML below is the same shape
            <code> agent_get_card</code> emits in production.
          </p>
        </div>

        <Card>
          <CardHeader>
            <CardTitle className="text-lg">Agent card (interactive preview)</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <McpCardIframe
              uri="ui://everruns/agent/agent_01933b5a000070008000000000000001/card"
              html={SAMPLE_AGENT_HTML}
              height={340}
              onAction={(msg) => setLog((prev) => [...prev, msg].slice(-20))}
            />

            <div className="rounded-md border bg-muted/30 p-3">
              <div className="mb-2 flex items-center justify-between">
                <div className="text-sm font-medium">Action log</div>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setLog([])}
                  disabled={log.length === 0}
                >
                  Clear
                </Button>
              </div>
              {log.length === 0 ? (
                <div className="text-xs text-muted-foreground">
                  Click a button on the card to see messages routed via postMessage.
                </div>
              ) : (
                <ol className="space-y-1 text-xs font-mono">
                  {log.map((msg, i) => (
                    <li key={i} className="break-all">
                      {JSON.stringify(msg)}
                    </li>
                  ))}
                </ol>
              )}
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
