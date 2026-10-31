import { viewerFor } from "../../../../server/auth";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

// Local session read. This is not an upstream proxy path, so it cannot reach
// the execution API or return the provider token.
export async function GET(request: Request) {
  try {
    const viewer = await viewerFor(request);
    if (!viewer) return new Response("Sign in to continue", { status: 401 });
    return Response.json(viewer, { headers: { "Cache-Control": "no-store" } });
  } catch {
    return new Response("Chat temporarily unavailable", { status: 503 });
  }
}
