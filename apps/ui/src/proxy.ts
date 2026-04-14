import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";
import { getLoginRedirectPath } from "@/lib/auth-redirect";

// Decision: proxy only enforces cookie presence for protected app routes.
// AuthProvider remains the source of truth for config bootstrap, user loading,
// token refresh, and auth-unavailable fallback UI.
export function proxy(request: NextRequest) {
  if (request.cookies.has("access_token")) {
    return NextResponse.next();
  }

  const loginPath = getLoginRedirectPath(request.nextUrl.pathname, request.nextUrl.searchParams);
  return NextResponse.redirect(new URL(loginPath, request.url));
}

export const config = {
  matcher: [
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
    "/orgs/:path*",
    "/sessions/:path*",
    "/settings/:path*",
    "/skills/:path*",
  ],
};
