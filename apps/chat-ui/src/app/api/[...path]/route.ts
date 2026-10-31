import { channelOf, settings, safeMutation, bootstrap } from "../../../server/config";
import { tokenFor, recoveryScope } from "../../../server/auth";
import { liveStream, proxyHeaders } from "../../../server/proxy";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
const allowed =
  /^(sessions(?:\/[a-zA-Z0-9_-]+(?:\/(?:messages|events|sse|cancel|question-answers|tool-approvals|files|work|settings|share))?)?|chat\/(?:main|config|shared\/[a-zA-Z0-9_-]+)|virtual-users\/me(?:\/connections(?:\/[a-zA-Z0-9_-]+(?:\/authorize)?)?)?)$/;
async function proxy(request: Request, { params }: { params: Promise<{ path: string[] }> }) {
  try {
    const path = (await params).path.join("/");
    if (path !== "chat/recovery" && !allowed.test(path))
      return new Response("Not found", { status: 404 });
    const channel = channelOf(request);
    if (path === "chat/config") {
      if (request.method !== "GET") return new Response("Method not allowed", { status: 405 });
      return Response.json(await bootstrap(channel), { headers: { "Cache-Control": "no-store" } });
    }
    if (path.startsWith("chat/shared/")) {
      if (request.method !== "GET") return new Response("Method not allowed", { status: 405 });
      const response = await fetch(
        `${settings().upstream}/v1/channels/${channel}/${path}${new URL(request.url).search}`,
        {
          cache: "no-store",
          redirect: "error",
          signal: AbortSignal.timeout(10000),
          // The private host sits behind an ingress that sets this chain.
          // Preserve it so the API can apply its trusted-proxy hop policy per viewer.
          headers: request.headers.has("x-forwarded-for")
            ? { "X-Forwarded-For": request.headers.get("x-forwarded-for")! }
            : undefined,
        },
      );
      return new Response(response.body, {
        status: response.status,
        headers: { "Content-Type": "application/json", "Cache-Control": "no-store" },
      });
    }
    if (request.method !== "GET" && !safeMutation(request))
      return new Response("Forbidden", { status: 403 });
    const token = await tokenFor(request);
    if (!token) return new Response("Sign in to continue", { status: 401 });
    if (path === "chat/recovery") {
      if (request.method !== "GET") return new Response("Method not allowed", { status: 405 });
      return Response.json(
        { scope: await recoveryScope(request) },
        { headers: { "Cache-Control": "no-store" } },
      );
    }
    const search = new URL(request.url).search;
    const route = path.startsWith("virtual-users/")
      ? `/v1/${path}`
      : `/v1/channels/${channel}/${path}`;
    const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
    const contentType = request.headers.get("content-type");
    if (contentType) headers["Content-Type"] = contentType;
    const lastId = request.headers.get("last-event-id");
    if (lastId) headers["Last-Event-ID"] = lastId;
    const idempotency = request.headers.get("idempotency-key");
    if (idempotency) headers["Idempotency-Key"] = idempotency;
    let body: ArrayBuffer | undefined;
    if (request.method !== "GET" && request.body) {
      const reader = request.body.getReader(),
        chunks: Uint8Array[] = [];
      let size = 0;
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        size += chunk.value.byteLength;
        if (size > 6 * 1024 * 1024) {
          await reader.cancel();
          return new Response("Request too large", { status: 413 });
        }
        chunks.push(chunk.value);
      }
      const bytes = new Uint8Array(size);
      let at = 0;
      for (const chunk of chunks) {
        bytes.set(chunk, at);
        at += chunk.byteLength;
      }
      body = bytes.buffer;
    }
    const upstream = await fetch(`${settings().upstream}${route}${search}`, {
      method: request.method,
      headers,
      body,
      // The live stream must outlive this proxy. Closing it here resets the
      // browser connection; other calls still fail closed after 30 seconds.
      signal: liveStream(path)
        ? request.signal
        : AbortSignal.any([request.signal, AbortSignal.timeout(30000)]),
      redirect: "error",
      cache: "no-store",
    });
    return new Response(upstream.body, {
      status: upstream.status,
      headers: proxyHeaders(path, upstream.headers.get("content-type")),
    });
  } catch {
    return new Response("Chat temporarily unavailable", { status: 503 });
  }
}
export { proxy as GET, proxy as POST, proxy as DELETE };
