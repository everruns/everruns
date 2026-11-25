import { channelOf, settings, safeMutation, bootstrap, runtimePath } from "../../../server/config";
import { tokenFor, recoveryScope } from "../../../server/auth";
import { liveStream, proxyHeaders } from "../../../server/proxy";
import { fetchUpstream } from "../../../server/upstream";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
const allowed =
  /^(sessions(?:\/[a-zA-Z0-9_-]+(?:\/(?:messages(?:\/[a-zA-Z0-9_-]+\/reactions\/(?!\.)[^/\s?#]{1,64})?|events|sse|cancel|question-answers|tool-approvals|files|work|settings|share|reactions))?)?|chat\/(?:main|config|view|plugins(?:\/[a-zA-Z0-9_-]+)?|shared\/[a-zA-Z0-9_-]+)|virtual-users\/me(?:\/connections(?:\/[a-zA-Z0-9_-]+(?:\/authorize)?)?)?)$/;
async function proxy(request: Request, { params }: { params: Promise<{ path: string[] }> }) {
  try {
    let segments = (await params).path;
    let selected: string | undefined;
    if (segments[0] === "chats" && segments.length > 2) {
      selected = segments[1];
      segments = segments.slice(2);
    }
    const path = segments.join("/");
    if (path !== "chat/recovery" && !allowed.test(path))
      return new Response("Not found", { status: 404 });
    let channel = await channelOf(request);
    const application = channel.startsWith("application_");
    let applicationToken: string | null = null;
    if (application) {
      applicationToken = await tokenFor(request);
      if (!applicationToken) return new Response("Sign in to continue", { status: 401 });
      if (selected) {
        const manifest = await fetch(`${settings().upstream}${runtimePath(channel)}/manifest`, {
          headers: { Authorization: `Bearer ${applicationToken}` },
          cache: "no-store",
          redirect: "error",
          signal: AbortSignal.timeout(10000),
        });
        if (!manifest.ok)
          return new Response("Application unavailable", { status: manifest.status });
        const resource = (await manifest.json()).resources?.find(
          (r: { kind: string; slug: string }) => r.kind === "chat" && r.slug === selected,
        );
        if (!resource || !/^appchan_[0-9a-f]{32}$/.test(resource.resource_id))
          return new Response("Not found", { status: 404 });
        channel = resource.resource_id;
      } else if (!path.startsWith("virtual-users/") && path !== "chat/recovery") {
        return new Response("Select a chat", { status: 404 });
      }
    } else if (selected) return new Response("Not found", { status: 404 });
    if (path === "chat/config") {
      if (request.method !== "GET") return new Response("Method not allowed", { status: 405 });
      const config = await bootstrap(channel);
      if (application) delete config.logo;
      return Response.json(config, { headers: { "Cache-Control": "no-store" } });
    }
    if (!application && path.startsWith("chat/shared/")) {
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
    const token = applicationToken ?? (await tokenFor(request));
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
      : // Segments arrive decoded; an emoji reaction segment must be re-encoded.
        `/v1/channels/${channel}/${path.split("/").map(encodeURIComponent).join("/")}`;
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
    const upstream = await fetchUpstream(
      `${settings().upstream}${route}${search}`,
      {
        method: request.method,
        headers,
        body,
        signal: request.signal,
        redirect: "error",
        cache: "no-store",
      },
      request.method === "GET" && liveStream(path),
    );
    return new Response(upstream.body, {
      status: upstream.status,
      headers: proxyHeaders(path, upstream.headers.get("content-type")),
    });
  } catch {
    return new Response("Chat temporarily unavailable", { status: 503 });
  }
}
export { proxy as GET, proxy as POST, proxy as PUT, proxy as DELETE };
