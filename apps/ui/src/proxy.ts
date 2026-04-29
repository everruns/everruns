import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";
import { getLoginRedirectPath } from "@/lib/auth-redirect";

const HOMEPAGE_DISCOVERY_LINKS = [
  '</api-doc/openapi.json>; rel="service-desc"; type="application/json"',
  '<https://docs.everruns.com/api/>; rel="service-doc"; type="text/html"',
].join(", ");

// Decision: proxy only enforces cookie presence for protected app routes.
// AuthProvider remains the source of truth for config bootstrap, user loading,
// token refresh, and auth-unavailable fallback UI.
export function proxy(request: NextRequest) {
  if (request.nextUrl.pathname === "/") {
    const response = NextResponse.next();
    response.headers.set("Link", HOMEPAGE_DISCOVERY_LINKS);
    return response;
  }

  if (request.cookies.has("access_token")) {
    return NextResponse.next();
  }

  const loginPath = getLoginRedirectPath(request.nextUrl.pathname, request.nextUrl.searchParams);
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
  ],
};
