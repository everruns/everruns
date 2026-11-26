import { channelOf, runtimePath, settings, safeMutation } from "../../../../server/config";
import { tokenFor } from "../../../../server/auth";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
// Only the Application manifest and its consumer adapters are reachable.
const allowed =
  /^(manifest|rooms\/[a-z][a-z0-9-]{1,46}[a-z0-9](?:\/messages|\/replies\/rmsg_[0-9a-f]{32})?|memory\/[a-z][a-z0-9-]{1,46}[a-z0-9])$/;
async function proxy(request: Request, { params }: { params: Promise<{ path: string[] }> }) {
  try {
    const path = (await params).path.join("/");
    if (!allowed.test(path)) return new Response("Not found", { status: 404 });
    const target = await channelOf(request);
    if (!target.startsWith("application_")) return new Response("Not found", { status: 404 });
    if (request.method !== "GET" && !safeMutation(request))
      return new Response("Forbidden", { status: 403 });
    const token = await tokenFor(request);
    if (!token) return new Response("Sign in to continue", { status: 401 });
    let body: string | undefined;
    if (request.method !== "GET" && request.method !== "DELETE") {
      const reader = request.body?.getReader();
      if (!reader) return new Response("Body required", { status: 400 });
      const chunks: Uint8Array[] = [];
      let size = 0;
      while (true) {
        const part = await reader.read();
        if (part.done) break;
        size += part.value.length;
        if (size > 2 * 1024 * 1024) {
          await reader.cancel();
          return new Response("Too large", { status: 413 });
        }
        chunks.push(part.value);
      }
      body = Buffer.concat(chunks).toString("utf8");
    }
    const response = await fetch(
      `${settings().upstream}${runtimePath(target)}/${path}${new URL(request.url).search}`,
      {
        method: request.method,
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body,
        cache: "no-store",
        redirect: "error",
        signal: AbortSignal.timeout(60000),
      },
    );
    return new Response(response.body, {
      status: response.status,
      headers: { "Content-Type": "application/json", "Cache-Control": "no-store" },
    });
  } catch {
    return new Response("Application temporarily unavailable", { status: 503 });
  }
}
export { proxy as GET, proxy as POST, proxy as PUT, proxy as DELETE };
