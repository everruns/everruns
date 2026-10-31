import { channelOf, settings } from "../../../../../server/config";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

const ID = /^avatar_[0-9a-f]{32}$/;

// Serves only this host's current public agent avatar. The id in the path is
// the cache key; a different id is not fetched.
export async function GET(request: Request, { params }: { params: Promise<{ id: string }> }) {
  const id = (await params).id;
  if (!ID.test(id)) return new Response("Not found", { status: 404 });
  try {
    const upstream = settings().upstream;
    const config = await fetch(
      `${upstream}/v1/channels/${channelOf(request)}/chat/config`,
      { cache: "no-store", signal: AbortSignal.timeout(10000), redirect: "error" },
    );
    if (!config.ok) return new Response("Not found", { status: 404 });
    const body = await config.json();
    const path = `/v1/avatars/${id}/circle-64.png`;
    if (body?.logo !== path) return new Response("Not found", { status: 404 });
    const image = await fetch(`${upstream}${path}`, {
      cache: "no-store",
      signal: AbortSignal.timeout(10000),
      redirect: "error",
    });
    const type = image.headers.get("content-type") ?? "";
    if (!image.ok || !type.startsWith("image/png")) return new Response("Not found", { status: 404 });
    const bytes = await image.arrayBuffer();
    if (bytes.byteLength === 0 || bytes.byteLength > 1024 * 1024)
      return new Response("Not found", { status: 404 });
    return new Response(bytes, {
      headers: {
        "Content-Type": "image/png",
        "Cache-Control": "public, max-age=31536000, immutable",
        "X-Content-Type-Options": "nosniff",
      },
    });
  } catch {
    return new Response("Not found", { status: 404 });
  }
}
