import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";
import { getLoginRedirectPath, isFullPageLoginRedirect } from "@/lib/auth-redirect";

const HOMEPAGE_DISCOVERY_LINKS = [
  '</api-doc/openapi.json>; rel="service-desc"; type="application/json"',
  '<https://docs.everruns.com/api/>; rel="service-doc"; type="text/html"',
].join(", ");

// Decision: proxy only enforces cookie presence for protected app routes.
// AuthProvider remains the source of truth for config bootstrap, user loading,
// token refresh, and auth-unavailable fallback UI.
function authDisabled(): boolean {
  return process.env.AUTH_MODE === "none";
}

// Next.js inlines direct `process.env.NAME` reads while compiling proxy code.
// Read through the environment object so standalone deployments can configure
// the trusted login origin at runtime instead of baking it into the image.
function configuredLoginOrigin(): string | undefined {
  const runtimeEnv = process.env;
  return runtimeEnv.AUTH_LOGIN_ORIGIN ?? runtimeEnv.NEXT_PUBLIC_AUTH_LOGIN_ORIGIN;
}

export function proxy(request: NextRequest) {
  if (request.nextUrl.pathname === "/") {
    const response = NextResponse.next();
    response.headers.set("Link", HOMEPAGE_DISCOVERY_LINKS);
    return response;
  }

  // A missing access token is not a missing session: the access cookie's
  // Max-Age is the short access-token lifetime (~15 min), while the refresh
  // cookie lives for weeks. Let refresh-only requests through so the client's
  // silent refresh (lib/api/client.ts) can re-mint the access token instead of
  // bouncing every idle tab to /login.
  if (
    authDisabled() ||
    request.cookies.has("access_token") ||
    request.cookies.has("refresh_token")
  ) {
    return NextResponse.next();
  }

  const loginPath = getLoginRedirectPath(
    request.nextUrl.pathname,
    request.nextUrl.searchParams,
    configuredLoginOrigin(),
  );
  if (isFullPageLoginRedirect(loginPath)) {
    // Preserve the configured absolute origin even when the reverse proxy's
    // upstream Host matches that origin; NextResponse.redirect may otherwise
    // collapse it to a relative Location header.
    return new NextResponse(null, { status: 307, headers: { Location: loginPath } });
  }
  return NextResponse.redirect(new URL(loginPath, request.url));
}

export const config = {
  matcher: [
    "/",
    "/agent-identities/:path*",
    "/agents/:path*",
    "/apps/:path*",
    "/capabilities/:path*",
    "/chat/:path*",
    "/chats/:path*",
    "/dashboard/:path*",
    "/durable/:path*",
    "/evals/:path*",
    "/harnesses/:path*",
    "/mcp-servers/:path*",
    "/models/:path*",
    "/orgs/:path*",
    "/sessions/:path*",
    "/settings/:path*",
    "/skills/:path*",
    "/memory/:path*",
  ],
};
