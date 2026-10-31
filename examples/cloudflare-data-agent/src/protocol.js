// The HTTP connection grants response-only, invocation-scoped authority. The
// endpoint never receives a CP user token or a custom-worker fleet credential.
const encoder = new TextEncoder();
export async function hmac(secret, bytes) {
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return [...new Uint8Array(await crypto.subtle.sign("HMAC", key, bytes))].map(b => b.toString(16).padStart(2, "0")).join("");
}

export async function authenticate(request, secret) {
  if (!secret || secret.length < 32 || request.method !== "POST") throw new Error("Unauthorized");
  const supplied = request.headers.get("X-Everruns-Signature") ?? "";
  if (!/^[0-9a-f]{64}$/.test(supplied)) throw new Error("Unauthorized");
  const reader = request.body?.getReader();
  if (!reader) throw new Error("Missing request body");
  const chunks = []; let size = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > 16 * 1024 * 1024) { await reader.cancel(); throw new Error("Request too large"); }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["verify"]);
  const signature = Uint8Array.from(supplied.match(/../g), b => parseInt(b, 16));
  if (!await crypto.subtle.verify("HMAC", key, signature, bytes)) throw new Error("Unauthorized");
  const body = JSON.parse(new TextDecoder().decode(bytes));
  if (body.protocol !== "everruns-worker/v1" || !Number.isInteger(body.issued_at) || Math.abs(Date.now() / 1000 - body.issued_at) > 60) throw new Error("Expired or unsupported request");
  if (body.audience !== request.url) throw new Error("Wrong endpoint audience");
  return body;
}

export function eventStream(body, execute) {
  const turn_id = body.invocation_id.replace(/^message_/, "turn_");
  const stream = new TransformStream();
  const writer = stream.writable.getWriter();
  let stopped = false;
  const abort = new AbortController();
  void writer.closed.catch(() => abort.abort());
  const emit = async (event_type, data, context = {}) => {
    if (stopped) throw new Error("Stream closed");
    await writer.write(encoder.encode(JSON.stringify({ type: event_type, ts: new Date().toISOString(), context: { turn_id, input_message_id: body.invocation_id, ...context }, data }) + "\n"));
  };
  const task = (async () => {
    try {
      await emit("turn.started", { turn_id, input_message_id: body.invocation_id, input_content: (body.input?.content ?? []).filter(p => p.type === "text").map(p => p.text).join("\n") });
      await execute(emit, turn_id, abort.signal);
    } catch (error) {
      console.error("External execution failed", { name: error instanceof Error ? error.name : "Error" });
      try { await emit("turn.failed", { turn_id, error: "External agent execution failed", error_code: "external_execution_failed" }); } catch { /* CP disconnected; no more authority. */ }
    } finally {
      stopped = true;
      try { await writer.close(); } catch { /* Consumer cancelled. */ }
    }
  })();
  return { response: new Response(stream.readable, { headers: { "Content-Type": "application/x-ndjson", "Cache-Control": "no-store" } }), task };
}
