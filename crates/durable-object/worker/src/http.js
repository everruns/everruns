import { timingSafeEqual } from "node:crypto";
import { HttpError, requireValue } from "./storage.js";

const MAX_BODY = 1024 * 1024;

export async function authenticate(request, token) {
  requireValue(
    typeof token === "string" && token.length >= 32,
    503,
    "authority service secret is not configured",
  );
  const hash = async (text) =>
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)),
    );
  requireValue(
    timingSafeEqual(
      await hash(request.headers.get("authorization") ?? ""),
      await hash(`Bearer ${token}`),
    ),
    401,
    "unauthorized",
  );
}

export function address(request, env) {
  const match = new URL(request.url).pathname.match(
    /^\/tenants\/([a-zA-Z0-9_-]{1,128})\/sessions\/(session_[a-f0-9]{32})\/([a-z][a-z_]{0,63})$/,
  );
  requireValue(
    match && match[1] === env.TENANT_ID,
    404,
    "unknown authority address",
  );
  return { tenant: match[1], sessionId: match[2], operation: match[3] };
}

export async function readJson(request) {
  requireValue(request.method === "POST", 405, "POST required");
  const reader = request.body?.getReader();
  requireValue(reader, 400, "JSON body required");
  const chunks = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      requireValue(size <= MAX_BODY, 413, "request body exceeds limit");
      chunks.push(value);
    }
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    const body = JSON.parse(new TextDecoder().decode(bytes));
    requireValue(
      body && typeof body === "object" && !Array.isArray(body),
      400,
      "JSON object required",
    );
    return body;
  } catch {
    throw new HttpError(400, "invalid JSON body");
  }
}

export const failure = (error) => {
  // Unexpected failures surface only as a generic 503 to the caller; keep the
  // cause in the Worker log so a deployment failure is diagnosable.
  if (!(error instanceof HttpError))
    console.error("authority operation failed", error);
  return Response.json(
    {
      error:
        error instanceof HttpError
          ? error.message
          : "authority operation failed",
    },
    { status: error instanceof HttpError ? error.status : 503 },
  );
};
