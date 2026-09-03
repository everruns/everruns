// A minimal MCP server that needs a human before it will answer.
//
// It speaks the stateless `2026-07-28` path (plain JSON-RPC POSTs, no session
// handshake) and answers `tools/call` with an MRTR `input_required` result
// carrying a URL mode elicitation: "send your user to this page". It only runs
// the tool once the caller reports that a human consented *and* the value it
// was waiting for has actually arrived on its own page.
//
// That is the whole point of URL mode: the API key below never travels through
// the MCP client or the model's context. It goes from the user's browser
// straight to this server.
//
// Run it:
//
//   PUBLIC_URL=https://elicit.example.com node elicit-server.mjs
//
// PUBLIC_URL must be an address the Everruns worker can reach and that passes
// its SSRF checks: a public https origin, not localhost and not a private
// range. A tunnel (ngrok, cloudflared) in front of this process works.
//
// GET /state reports what the server actually holds, so you can prove the key
// landed here and nowhere else.
import { createServer } from "node:http";

const PORT = Number(process.env.PORT ?? 8391);
// Origin the elicitation URL points at. Defaults to loopback for a first look;
// override it with the address Everruns will reach.
const PUBLIC_URL = (process.env.PUBLIC_URL ?? `http://localhost:${PORT}`).replace(/\/$/, "");

const state = { calls: 0, elicitations: 0, accepted: false, key: null };

const mask = (value) =>
  value && value.length > 8
    ? `${value.slice(0, 7)}${"•".repeat(8)}${value.slice(-4)}`
    : "••••";

const json = (res, body, status = 200) => {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
};

const TOOLS = [
  {
    name: "run_revenue_report",
    description:
      "Run a revenue report for a given month. Requires the user's own API key, which this server collects directly from them.",
    inputSchema: {
      type: "object",
      properties: {
        month: { type: "string", description: "Month to report on, e.g. 2026-08" },
      },
      required: ["month"],
    },
  },
];

const page = (body) => `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Example Analytics — connect</title><style>
body{font:15px/1.55 system-ui,-apple-system,sans-serif;margin:0;display:grid;
place-items:center;min-height:100vh;background:#f5f6f8;color:#14161a}
.card{background:#fff;border:1px solid #e2e4e9;border-radius:14px;padding:30px 34px;width:460px}
h1{font-size:18px;margin:0 0 6px}p{color:#5c6270;font-size:13.5px;margin:0 0 18px}
label{display:block;font-weight:600;font-size:13px;margin:0 0 6px}
input{width:100%;padding:11px 12px;font:14px ui-monospace,monospace;border:1px solid #ccd0d8;
border-radius:8px;box-sizing:border-box}
button{margin-top:18px;width:100%;background:#14161a;color:#fff;border:0;border-radius:8px;
padding:12px 16px;font:600 14px system-ui;cursor:pointer}
.ok{color:#12793f}code{font:13px ui-monospace,monospace;background:#f0f1f4;padding:2px 6px;border-radius:5px}
</style></head><body><div class="card" id="c">${body}</div></body></html>`;

const CONNECT_PAGE = page(`<h1>Connect your account</h1>
<p>Paste your API key so we can run the report the agent asked for. This page is
served by us — the key never reaches the agent that sent you here.</p>
<label for="key">API key</label>
<input id="key" type="password" autocomplete="off" spellcheck="false" autofocus placeholder="sk-live-…">
<button onclick="save()">Save key</button>
<script>
async function save(){
  const key = document.getElementById('key').value;
  const res = await fetch('/save', {method:'POST', headers:{'content-type':'application/json'},
    body: JSON.stringify({key})});
  const data = await res.json();
  document.getElementById('c').innerHTML =
    '<h1 class="ok">Key saved</h1><p>Stored as <code>' + data.masked +
    '</code>. You can close this tab and tell the agent you are done.</p>';
}
</script>`);

function elicitation(id) {
  state.elicitations += 1;
  return {
    jsonrpc: "2.0",
    id,
    result: {
      resultType: "input_required",
      // Opaque to the client: it must be echoed back verbatim on the retry.
      requestState: `example-state-${state.elicitations}`,
      inputRequests: {
        api_key: {
          method: "elicitation/create",
          params: {
            mode: "url",
            url: `${PUBLIC_URL}/connect?ref=rev-2026-08`,
            message:
              "Example Analytics needs your API key before it can run this report. Enter it on our own page — it is never sent through the agent.",
          },
        },
      },
    },
  };
}

createServer((req, res) => {
  const url = new URL(req.url, PUBLIC_URL);

  if (url.pathname === "/connect") {
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    return res.end(CONNECT_PAGE);
  }

  if (url.pathname === "/save" && req.method === "POST") {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      const { key } = JSON.parse(body || "{}");
      state.key = key || null;
      json(res, { ok: true, masked: mask(key || "") });
    });
    return;
  }

  if (url.pathname === "/state") {
    return json(res, {
      calls: state.calls,
      elicitations: state.elicitations,
      accepted: state.accepted,
      key_received: Boolean(state.key),
      key_masked: state.key ? mask(state.key) : null,
    });
  }

  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", () => {
    let rpc;
    try {
      rpc = JSON.parse(body || "{}");
    } catch {
      return json(res, { error: "bad json" }, 400);
    }
    const { id, method, params } = rpc;

    if (method === "initialize") {
      return json(res, {
        jsonrpc: "2.0",
        id,
        result: {
          protocolVersion: "2026-07-28",
          capabilities: { tools: {} },
          serverInfo: { name: "example-analytics", version: "0.1.0" },
        },
      });
    }
    if (method === "notifications/initialized") return json(res, {});
    if (method === "tools/list") {
      return json(res, { jsonrpc: "2.0", id, result: { tools: TOOLS } });
    }
    if (method === "tools/call") {
      state.calls += 1;
      const accepted = Object.values(params?.inputResponses ?? {}).some(
        (response) => response?.action === "accept",
      );
      if (accepted) state.accepted = true;

      // Both halves are required: a human said yes, and the value is here.
      // Consent alone means they clicked; the key alone means someone filled
      // the form without the agent ever being told.
      if (accepted && state.key) {
        const month = params?.arguments?.month ?? "2026-08";
        return json(res, {
          jsonrpc: "2.0",
          id,
          result: {
            resultType: "complete",
            content: [
              {
                type: "text",
                text:
                  `Revenue report for ${month}: $412,800 across 1,284 accounts, ` +
                  "up 6.2% month over month. Top segment: self-serve ($188,400).",
              },
            ],
            isError: false,
          },
        });
      }
      return json(res, elicitation(id));
    }

    return json(res, {
      jsonrpc: "2.0",
      id,
      error: { code: -32601, message: `unknown method ${method}` },
    });
  });
}).listen(PORT, "0.0.0.0", () => {
  console.log(`MCP endpoint:   POST ${PUBLIC_URL}/`);
  console.log(`Elicited page:  GET  ${PUBLIC_URL}/connect`);
  console.log(`Server state:   GET  ${PUBLIC_URL}/state`);
});
