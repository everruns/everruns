import { LoginRateLimitError } from "../../../server/store";
import { authenticate, logout } from "../../../server/auth";
import { safeMutation } from "../../../server/config";
export const runtime = "nodejs";
export async function GET(request: Request, { params }: { params: Promise<{ action: string }> }) {
  try {
    return await authenticate(request, (await params).action);
  } catch (error) {
    if (error instanceof LoginRateLimitError)
      return new Response("Too many sign-in attempts. Try again later.", {
        status: 429,
        headers: { "Cache-Control": "no-store", "Retry-After": "600" },
      });
    return new Response("Sign-in did not complete. Return to your chat and try again.", {
      status: 400,
      headers: { "Cache-Control": "no-store" },
    });
  }
}
export async function POST(request: Request, { params }: { params: Promise<{ action: string }> }) {
  if (!safeMutation(request) || (await params).action !== "logout")
    return new Response("Forbidden", { status: 403 });
  await logout();
  return new Response(null, { status: 204 });
}
