// Integration snippet generators.
//
// Everruns is a headless platform: people compose Agents / Harnesses / Apps in
// this management UI but invoke them from their own applications over the HTTP
// API. These pure functions turn a composed entity into copy-paste integration
// material (curl / TypeScript / Python) plus a ready-to-paste prompt for a
// coding agent. Keeping them pure (string in, string out) makes them trivially
// unit-testable and reusable across the Agent, Harness, and App surfaces.

export interface CodeSample {
  /** Tab label, e.g. "curl", "TypeScript", "Python". */
  label: string;
  /** Highlight hint / fenced-code language, e.g. "bash", "ts", "python". */
  language: string;
  code: string;
}

/** Placeholder token rendered in snippets; users swap in a real PAT. */
export const TOKEN_PLACEHOLDER = "evr_pat_xxx";

const FALLBACK_ORIGIN = "https://your-everruns-host";

function resolveOrigin(origin: string): string {
  return origin || FALLBACK_ORIGIN;
}

/** REST base URL. The UI mounts the backend under `/api`; public base is `/api/v1`. */
export function apiBaseUrl(origin: string): string {
  return `${resolveOrigin(origin)}/api/v1`;
}

/** OpenAPI document URL — the source of truth external integrators should read. */
export function openApiUrl(origin: string): string {
  return `${resolveOrigin(origin)}/api-doc/openapi.json`;
}

/** Personal-access-token management page in this UI. */
export function tokensUrl(origin: string): string {
  return `${resolveOrigin(origin)}/settings/personal-access-tokens`;
}

/** Make a possibly-relative endpoint absolute against the given origin. */
function absolutize(endpointUrl: string, origin: string): string {
  if (endpointUrl.startsWith("http://") || endpointUrl.startsWith("https://")) {
    return endpointUrl;
  }
  return `${resolveOrigin(origin)}${endpointUrl.startsWith("/") ? "" : "/"}${endpointUrl}`;
}

type SessionBinding =
  | { agentId: string; harnessId?: undefined }
  | { harnessId: string; agentId?: undefined };

/**
 * Code samples for the core direct-API flow: create a session bound to this
 * agent (or harness), send a user message, then stream the response over SSE.
 *
 * Generated JS uses string concatenation (not template literals) and bash uses
 * unbraced `$VARS` so nothing in the emitted code collides with this file's own
 * `${}` interpolation.
 */
export function sessionFlowSamples(opts: { origin: string } & SessionBinding): CodeSample[] {
  const base = apiBaseUrl(opts.origin);
  const isAgent = opts.agentId !== undefined;
  const kind = isAgent ? "agent" : "harness";
  const bindKey = isAgent ? "agent_id" : "harness_id";
  const bindVal = isAgent ? opts.agentId! : opts.harnessId!;
  const bindJson = `{"${bindKey}": "${bindVal}"}`;
  const bindJs = `{ ${bindKey}: "${bindVal}" }`;
  const bindPy = `{"${bindKey}": "${bindVal}"}`;

  const curl: CodeSample = {
    label: "curl",
    language: "bash",
    code: [
      `# 1. Credentials — create a token at ${tokensUrl(opts.origin)}`,
      `export EVERRUNS_TOKEN="${TOKEN_PLACEHOLDER}"`,
      `export EVERRUNS_API="${base}"`,
      ``,
      `# 2. Create a session bound to this ${kind}`,
      `SESSION_ID=$(curl -s -X POST "$EVERRUNS_API/sessions" \\`,
      `  -H "Authorization: Bearer $EVERRUNS_TOKEN" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '${bindJson}' | jq -r .id)`,
      ``,
      `# 3. Send a user message (triggers a turn)`,
      `curl -s -X POST "$EVERRUNS_API/sessions/$SESSION_ID/messages" \\`,
      `  -H "Authorization: Bearer $EVERRUNS_TOKEN" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{"message":{"role":"user","content":[{"type":"text","text":"Hello!"}]}}'`,
      ``,
      `# 4. Stream the response (Server-Sent Events)`,
      `curl -N "$EVERRUNS_API/sessions/$SESSION_ID/sse" \\`,
      `  -H "Authorization: Bearer $EVERRUNS_TOKEN"`,
    ].join("\n"),
  };

  const typescript: CodeSample = {
    label: "TypeScript",
    language: "ts",
    code: [
      `const API = "${base}";`,
      `const TOKEN = process.env.EVERRUNS_TOKEN!; // ${TOKEN_PLACEHOLDER}`,
      `const headers = {`,
      `  Authorization: "Bearer " + TOKEN,`,
      `  "Content-Type": "application/json",`,
      `};`,
      ``,
      `// 1. Create a session bound to this ${kind}`,
      `const session = await fetch(API + "/sessions", {`,
      `  method: "POST",`,
      `  headers,`,
      `  body: JSON.stringify(${bindJs}),`,
      `}).then((r) => r.json());`,
      ``,
      `// 2. Send a user message`,
      `await fetch(API + "/sessions/" + session.id + "/messages", {`,
      `  method: "POST",`,
      `  headers,`,
      `  body: JSON.stringify({`,
      `    message: { role: "user", content: [{ type: "text", text: "Hello!" }] },`,
      `  }),`,
      `});`,
      ``,
      `// 3. Stream events until the turn completes`,
      `const res = await fetch(API + "/sessions/" + session.id + "/sse", { headers });`,
      `const reader = res.body!.getReader();`,
      `const decoder = new TextDecoder();`,
      `for (;;) {`,
      `  const { value, done } = await reader.read();`,
      `  if (done) break;`,
      `  // Watch the stream for "output.message.completed" (final answer).`,
      `  process.stdout.write(decoder.decode(value));`,
      `}`,
    ].join("\n"),
  };

  const python: CodeSample = {
    label: "Python",
    language: "python",
    code: [
      `import requests`,
      ``,
      `API = "${base}"`,
      `TOKEN = "${TOKEN_PLACEHOLDER}"  # create at ${tokensUrl(opts.origin)}`,
      `headers = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}`,
      ``,
      `# 1. Create a session bound to this ${kind}`,
      `session = requests.post(`,
      `    f"{API}/sessions", headers=headers, json=${bindPy}`,
      `).json()`,
      `session_id = session["id"]`,
      ``,
      `# 2. Send a user message`,
      `requests.post(`,
      `    f"{API}/sessions/{session_id}/messages",`,
      `    headers=headers,`,
      `    json={"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]}},`,
      `)`,
      ``,
      `# 3. Stream events until the turn completes`,
      `with requests.get(f"{API}/sessions/{session_id}/sse", headers=headers, stream=True) as r:`,
      `    for line in r.iter_lines():`,
      `        if line:`,
      `            # Watch the stream for "output.message.completed" (final answer).`,
      `            print(line.decode())`,
    ].join("\n"),
  };

  const rust: CodeSample = {
    label: "Rust",
    language: "rust",
    code: [
      `// Cargo.toml: reqwest = { version = "0.12", features = ["json", "stream"] }`,
      `//            tokio = { version = "1", features = ["full"] }; futures-util = "0.3"`,
      `use futures_util::StreamExt;`,
      ``,
      `#[tokio::main]`,
      `async fn main() -> Result<(), Box<dyn std::error::Error>> {`,
      `    let api = "${base}";`,
      `    let token = "${TOKEN_PLACEHOLDER}";`,
      `    let http = reqwest::Client::new();`,
      ``,
      `    // 1. Create a session bound to this ${kind}`,
      `    let session: serde_json::Value = http`,
      `        .post(format!("{api}/sessions"))`,
      `        .bearer_auth(token)`,
      `        .json(&serde_json::json!({ "${bindKey}": "${bindVal}" }))`,
      `        .send().await?.json().await?;`,
      `    let session_id = session["id"].as_str().unwrap();`,
      ``,
      `    // 2. Send a user message`,
      `    http.post(format!("{api}/sessions/{session_id}/messages"))`,
      `        .bearer_auth(token)`,
      `        .json(&serde_json::json!({`,
      `            "message": { "role": "user", "content": [{ "type": "text", "text": "Hello!" }] }`,
      `        }))`,
      `        .send().await?;`,
      ``,
      `    // 3. Stream events until the turn completes`,
      `    let mut stream = http`,
      `        .get(format!("{api}/sessions/{session_id}/sse"))`,
      `        .bearer_auth(token)`,
      `        .send().await?`,
      `        .bytes_stream();`,
      `    while let Some(chunk) = stream.next().await {`,
      `        print!("{}", String::from_utf8_lossy(&chunk?)); // watch for output.message.completed`,
      `    }`,
      `    Ok(())`,
      `}`,
    ].join("\n"),
  };

  return [curl, typescript, python, rust];
}

/**
 * Code samples using the official Everruns SDKs (Python / TypeScript / Rust).
 * The SDKs add typed models and SSE streaming with automatic reconnection. The
 * SDK base URL is the `/api` root (the client appends the version itself).
 */
export function sdkSessionSamples(opts: { origin: string } & SessionBinding): CodeSample[] {
  const sdkBase = `${resolveOrigin(opts.origin)}/api`;
  const isAgent = opts.agentId !== undefined;
  const kind = isAgent ? "agent" : "harness";
  const bindKey = isAgent ? "agent_id" : "harness_id";
  const bindVal = isAgent ? opts.agentId! : opts.harnessId!;

  const python: CodeSample = {
    label: "Python",
    language: "python",
    code: [
      `# pip install everruns-sdk`,
      `import asyncio`,
      `from everruns_sdk import Everruns`,
      ``,
      `async def main():`,
      `    client = Everruns(api_key="${TOKEN_PLACEHOLDER}", base_url="${sdkBase}")`,
      ``,
      `    # 1. Start a session bound to this ${kind}`,
      `    session = await client.sessions.create(${bindKey}="${bindVal}")`,
      ``,
      `    # 2. Send a user message`,
      `    await client.messages.create(session.id, "Hello!")`,
      ``,
      `    # 3. Stream the response (auto-reconnecting SSE)`,
      `    async for event in client.events.stream(session.id):`,
      `        if event.type == "output.message.delta":`,
      `            print(event.data.get("delta", ""), end="", flush=True)`,
      `        elif event.type == "turn.completed":`,
      `            break`,
      `    await client.close()`,
      ``,
      `asyncio.run(main())`,
    ].join("\n"),
  };

  const typescript: CodeSample = {
    label: "TypeScript",
    language: "ts",
    code: [
      `// npm install @everruns/sdk`,
      `import { Client } from "@everruns/sdk";`,
      ``,
      `const client = new Client({ apiKey: "${TOKEN_PLACEHOLDER}", baseUrl: "${sdkBase}" });`,
      ``,
      `// 1. Start a session bound to this ${kind}`,
      `const session = await client.sessions.create({ ${bindKey}: "${bindVal}" });`,
      ``,
      `// 2. Send a user message`,
      `await client.messages.create(session.id, "Hello!");`,
      ``,
      `// 3. Stream the response (auto-reconnecting SSE)`,
      `for await (const event of client.events.stream(session.id)) {`,
      `  if (event.type === "output.message.delta") {`,
      `    process.stdout.write(event.data.delta ?? "");`,
      `  } else if (event.type === "turn.completed") {`,
      `    break;`,
      `  }`,
      `}`,
    ].join("\n"),
  };

  const rust: CodeSample = {
    label: "Rust",
    language: "rust",
    code: [
      `// cargo add everruns-sdk`,
      `use everruns_sdk::Client;`,
      `use futures_util::StreamExt;`,
      ``,
      `#[tokio::main]`,
      `async fn main() -> Result<(), Box<dyn std::error::Error>> {`,
      `    let client = Client::new("${TOKEN_PLACEHOLDER}").with_base_url("${sdkBase}");`,
      ``,
      `    // 1. Start a session bound to this ${kind}`,
      `    let session = client.sessions().create_for_${kind}("${bindVal}").await?;`,
      ``,
      `    // 2. Send a user message`,
      `    client.messages().create(&session.id, "Hello!").await?;`,
      ``,
      `    // 3. Stream the response (auto-reconnecting SSE)`,
      `    let mut events = client.events().stream(&session.id).await?;`,
      `    while let Some(event) = events.next().await {`,
      `        let event = event?;`,
      `        if event.type_ == "turn.completed" { break; }`,
      `    }`,
      `    Ok(())`,
      `}`,
    ].join("\n"),
  };

  return [python, typescript, rust];
}

/** Code samples for invoking an App webhook channel. */
export function webhookSamples(opts: { origin: string; endpointUrl: string }): CodeSample[] {
  const url = absolutize(opts.endpointUrl, opts.origin);
  return [
    {
      label: "curl",
      language: "bash",
      code: [
        `curl -X POST "${url}" \\`,
        `  -H "Authorization: Bearer YOUR_WEBHOOK_TOKEN" \\`,
        `  -H "Content-Type: application/json" \\`,
        `  -d '{"example": "payload"}'`,
      ].join("\n"),
    },
    {
      label: "TypeScript",
      language: "ts",
      code: [
        `await fetch("${url}", {`,
        `  method: "POST",`,
        `  headers: {`,
        `    Authorization: "Bearer YOUR_WEBHOOK_TOKEN",`,
        `    "Content-Type": "application/json",`,
        `  },`,
        `  body: JSON.stringify({ example: "payload" }),`,
        `});`,
      ].join("\n"),
    },
    {
      label: "Python",
      language: "python",
      code: [
        `import requests`,
        ``,
        `requests.post(`,
        `    "${url}",`,
        `    headers={"Authorization": "Bearer YOUR_WEBHOOK_TOKEN"},`,
        `    json={"example": "payload"},`,
        `)`,
      ].join("\n"),
    },
    {
      label: "Rust",
      language: "rust",
      code: [
        `// reqwest = { version = "0.12", features = ["json"] }`,
        `reqwest::Client::new()`,
        `    .post("${url}")`,
        `    .bearer_auth("YOUR_WEBHOOK_TOKEN")`,
        `    .json(&serde_json::json!({ "example": "payload" }))`,
        `    .send().await?;`,
      ].join("\n"),
    },
  ];
}

/** Code samples for invoking an App A2A channel (JSON-RPC 2.0 message/send). */
export function a2aSamples(opts: { origin: string; endpointUrl: string }): CodeSample[] {
  const url = absolutize(opts.endpointUrl, opts.origin);
  const rpcJson = [
    `{`,
    `  "jsonrpc": "2.0",`,
    `  "id": "1",`,
    `  "method": "message/send",`,
    `  "params": {`,
    `    "message": {`,
    `      "role": "user",`,
    `      "parts": [{ "kind": "text", "text": "Hello!" }],`,
    `      "messageId": "msg-1"`,
    `    }`,
    `  }`,
    `}`,
  ].join("\n");

  return [
    {
      label: "curl",
      language: "bash",
      code: [
        `curl -X POST "${url}" \\`,
        `  -H "Authorization: Bearer YOUR_A2A_KEY" \\`,
        `  -H "Content-Type: application/json" \\`,
        `  -d '${rpcJson.replace(/\n/g, "")}'`,
      ].join("\n"),
    },
    {
      label: "TypeScript",
      language: "ts",
      code: [
        `await fetch("${url}", {`,
        `  method: "POST",`,
        `  headers: {`,
        `    Authorization: "Bearer YOUR_A2A_KEY",`,
        `    "Content-Type": "application/json",`,
        `  },`,
        `  body: JSON.stringify({`,
        `    jsonrpc: "2.0",`,
        `    id: "1",`,
        `    method: "message/send",`,
        `    params: {`,
        `      message: {`,
        `        role: "user",`,
        `        parts: [{ kind: "text", text: "Hello!" }],`,
        `        messageId: "msg-1",`,
        `      },`,
        `    },`,
        `  }),`,
        `});`,
      ].join("\n"),
    },
    {
      label: "Python",
      language: "python",
      code: [
        `import requests`,
        ``,
        `requests.post(`,
        `    "${url}",`,
        `    headers={"Authorization": "Bearer YOUR_A2A_KEY"},`,
        `    json={`,
        `        "jsonrpc": "2.0",`,
        `        "id": "1",`,
        `        "method": "message/send",`,
        `        "params": {`,
        `            "message": {`,
        `                "role": "user",`,
        `                "parts": [{"kind": "text", "text": "Hello!"}],`,
        `                "messageId": "msg-1",`,
        `            }`,
        `        },`,
        `    },`,
        `)`,
      ].join("\n"),
    },
    {
      label: "Rust",
      language: "rust",
      code: [
        `// reqwest = { version = "0.12", features = ["json"] }`,
        `reqwest::Client::new()`,
        `    .post("${url}")`,
        `    .bearer_auth("YOUR_A2A_KEY")`,
        `    .json(&serde_json::json!({`,
        `        "jsonrpc": "2.0",`,
        `        "id": "1",`,
        `        "method": "message/send",`,
        `        "params": { "message": {`,
        `            "role": "user",`,
        `            "parts": [{ "kind": "text", "text": "Hello!" }],`,
        `            "messageId": "msg-1"`,
        `        } }`,
        `    }))`,
        `    .send().await?;`,
      ].join("\n"),
    },
  ];
}

export type PromptKind = "agent" | "harness" | "webhook" | "a2a";

function promptData(value: string): string {
  return JSON.stringify(value);
}

/**
 * A ready-to-paste prompt handed to a coding agent (Claude Code, Cursor, ...) so
 * it can wire up the integration directly in the user's codebase. It states the
 * endpoint, auth, exact flow, and links the OpenAPI spec for precise schemas.
 */
export function codingAgentPrompt(opts: {
  origin: string;
  kind: PromptKind;
  id?: string;
  name?: string;
  endpointUrl?: string;
}): string {
  const subject: Record<PromptKind, string> = {
    agent: "an Everruns agent",
    harness: "an Everruns harness",
    webhook: "an Everruns app (webhook channel)",
    a2a: "an Everruns app (A2A channel)",
  };
  const intro = [
    `I want to integrate ${subject[opts.kind]} into my application.`,
    ``,
    `Everruns is a headless agent platform that I call over its HTTP API.`,
    `- OpenAPI spec (authoritative schemas): ${openApiUrl(opts.origin)}`,
  ];

  if (opts.kind === "agent" || opts.kind === "harness") {
    const bindKey = opts.kind === "agent" ? "agent_id" : "harness_id";
    return [
      ...intro,
      `- Base URL: ${apiBaseUrl(opts.origin)}`,
      `- Auth: header \`Authorization: Bearer <token>\` (a personal access token, prefix \`evr_pat_\`).`,
      ``,
      `Target ${opts.kind}:`,
      `- ${bindKey}: ${opts.id ?? "<id>"}${opts.name ? `\n- name: ${promptData(opts.name)}` : ""}`,
      ``,
      `Integration flow:`,
      `1. POST /sessions with body {"${bindKey}": "${opts.id ?? "<id>"}"} -> returns a session; keep its "id".`,
      `2. POST /sessions/{id}/messages with body`,
      `   {"message": {"role": "user", "content": [{"type": "text", "text": "<user text>"}]}}`,
      `   to trigger a turn.`,
      `3. GET /sessions/{id}/sse (Server-Sent Events) to stream the response. Watch for`,
      `   "output.message.completed" (final agent message) and "session.idled" (turn done).`,
      `   Alternatively poll GET /sessions/{id}/messages.`,
      ``,
      `Official SDKs are available — prefer them if my project uses one of these:`,
      `- Rust: \`cargo add everruns-sdk\``,
      `- Python: \`pip install everruns-sdk\``,
      `- TypeScript: \`npm install @everruns/sdk\``,
      `They expose sub-clients (sessions, messages, events) and SSE streaming with`,
      `automatic reconnection. The SDK base URL is the \`/api\` root (it appends the`,
      `version). Otherwise call the REST API directly per the flow above.`,
      ``,
      `Please add a small typed client to my codebase that creates a session once,`,
      `sends a user message, waits for the final agent message, and returns it as a`,
      `string. Read the OpenAPI spec above for exact request/response schemas.`,
    ].join("\n");
  }

  if (opts.kind === "webhook") {
    return [
      ...intro,
      ``,
      `I have an Everruns App with a webhook channel:`,
      `- Endpoint: POST ${absolutize(opts.endpointUrl ?? "", opts.origin)}`,
      `- Auth: header \`Authorization: Bearer <webhook-token>\` (or \`X-Everruns-Webhook-Token\`).`,
      ``,
      `Please add code to my application that POSTs a JSON payload to this endpoint`,
      `whenever <my trigger> happens. The payload fields are referenced by the app's`,
      `message template, so include the fields my template expects. The endpoint`,
      `returns 202 with a session id. Use my project's existing language and HTTP library.`,
    ].join("\n");
  }

  // a2a
  return [
    ...intro,
    ``,
    `I have an Everruns App exposed as an A2A (Agent2Agent) agent:`,
    `- JSON-RPC endpoint: POST ${absolutize(opts.endpointUrl ?? "", opts.origin)}`,
    `- Auth: header \`Authorization: Bearer <api-key>\`.`,
    `- Protocol: A2A JSON-RPC 2.0, method "message/send" with a user message part.`,
    ``,
    `Please add an A2A client to my application that sends a text message to this`,
    `agent and reads the resulting task/message back. Use my project's existing`,
    `language and HTTP library.`,
  ].join("\n");
}
