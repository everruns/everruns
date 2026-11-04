// The `everruns-worker/v1` request contract. The HTTP connection grants
// response-only, invocation-scoped authority. The endpoint never
// receives a CP user token or a custom-worker fleet credential.
const encoder = new TextEncoder();
const MAX_BODY_BYTES = 16 * 1024 * 1024;
const MAX_CLOCK_SKEW_SECONDS = 60;

export async function hmac(secret, bytes) {
  const key = await hmacKey(secret, "sign");
  const signature = await crypto.subtle.sign("HMAC", key, bytes);
  return [...new Uint8Array(signature)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// Verifies the HMAC over the exact body bytes, the timestamp and the
// endpoint audience, then returns the parsed body.
export async function authenticate(request, secret) {
  if (!secret || secret.length < 32 || request.method !== "POST") {
    throw new Error("Unauthorized");
  }
  const supplied = request.headers.get("X-Everruns-Signature") ?? "";
  if (!/^[0-9a-f]{64}$/.test(supplied)) throw new Error("Unauthorized");

  const bytes = await readBody(request);
  const key = await hmacKey(secret, "verify");
  const signature = Uint8Array.from(supplied.match(/../g), (b) =>
    parseInt(b, 16),
  );
  if (!(await crypto.subtle.verify("HMAC", key, signature, bytes))) {
    throw new Error("Unauthorized");
  }

  const body = JSON.parse(new TextDecoder().decode(bytes));
  const age = Math.abs(Date.now() / 1000 - body.issued_at);
  if (
    body.protocol !== "everruns-worker/v1" ||
    !Number.isInteger(body.issued_at) ||
    age > MAX_CLOCK_SKEW_SECONDS
  ) {
    throw new Error("Expired or unsupported request");
  }
  if (body.audience !== request.url) {
    throw new Error("Wrong endpoint audience");
  }
  return body;
}

// Streams newline-delimited canonical events back to Everruns while
// `execute(emit, turn_id, signal)` runs. Any error becomes
// `turn.failed` without its diagnostic text.
export function eventStream(body, execute) {
  const turn_id = body.invocation_id.replace(/^message_/, "turn_");
  const stream = new TransformStream();
  const writer = stream.writable.getWriter();
  const abort = new AbortController();
  void writer.closed.catch(() => abort.abort());
  let stopped = false;

  const emit = async (type, data, context = {}) => {
    if (stopped) throw new Error("Stream closed");
    const event = {
      type,
      ts: new Date().toISOString(),
      context: {
        turn_id,
        input_message_id: body.invocation_id,
        ...context,
      },
      data,
    };
    await writer.write(encoder.encode(JSON.stringify(event) + "\n"));
  };

  const task = (async () => {
    try {
      await emit("turn.started", {
        turn_id,
        input_message_id: body.invocation_id,
        input_content: textOf(body.input ?? {}),
      });
      await execute(emit, turn_id, abort.signal);
    } catch (error) {
      console.error("External execution failed", {
        name: error instanceof Error ? error.name : "Error",
      });
      try {
        await emit("turn.failed", {
          turn_id,
          error: "External agent execution failed",
          error_code: "external_execution_failed",
        });
      } catch {
        // CP disconnected; no more authority.
      }
    } finally {
      stopped = true;
      try {
        await writer.close();
      } catch {
        // Consumer cancelled.
      }
    }
  })();

  const response = new Response(stream.readable, {
    headers: {
      "Content-Type": "application/x-ndjson",
      "Cache-Control": "no-store",
    },
  });
  return { response, task };
}

// Message helpers shared by the agent code.

export const newId = (prefix) =>
  `${prefix}_${crypto.randomUUID().replaceAll("-", "")}`;

export const textOf = (message) =>
  (message.content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n");

export const runtimeMessage = (role, text) => ({
  id: newId("message"),
  role,
  content: [{ type: "text", text }],
  created_at: new Date().toISOString(),
});

function hmacKey(secret, usage) {
  return crypto.subtle.importKey(
    "raw",
    encoder.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    [usage],
  );
}

async function readBody(request) {
  const reader = request.body?.getReader();
  if (!reader) throw new Error("Missing request body");
  const chunks = [];
  let size = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > MAX_BODY_BYTES) {
      await reader.cancel();
      throw new Error("Request too large");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}
