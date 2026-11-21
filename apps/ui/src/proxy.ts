import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";
import { getLoginRedirectPath, isFullPageLoginRedirect } from "@/lib/auth-redirect";

const HOMEPAGE_DISCOVERY_LINKS = [
  '</api-doc/openapi.json>; rel="service-desc"; type="application/json"',
  '<https://everruns.com/docs/api/>; rel="service-doc"; type="text/html"',
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

async function homepageMarkdown(url: string, method: string): Promise<Response> {
  // THREAT[TM-WEB-019]: Fetch the public discovery document from the configured API. The
  // browser's cookies, authorization, and Host never go to this request.
  try {
    const response = await fetch(url, {
      method,
      redirect: "error",
      signal: AbortSignal.timeout(3000),
    });
    if (response.ok) {
      const headers = new Headers({ Vary: "Accept" });
      for (const name of ["Content-Type", "Cache-Control"]) {
        const value = response.headers.get(name);
        if (value) headers.set(name, value);
      }
      return new Response(response.body, { headers });
    }
    await response.body?.cancel();
  } catch {
    // Keep the browser homepage independent of the discovery endpoint.
  }
  return new Response(null, { status: 502, headers: { Vary: "Accept" } });
}

export async function proxy(request: NextRequest) {
  if (request.nextUrl.pathname === "/") {
    const runtimeEnv = process.env;
    const markdownUrl = runtimeEnv.UI_HOMEPAGE_MARKDOWN_URL;
    if (
      markdownUrl &&
      (request.method === "GET" || request.method === "HEAD") &&
      request.headers
        .get("Accept")
        ?.split(",")
        .some((value) => {
          const [mediaType, ...parameters] = value.split(";");
          return (
            mediaType.trim().toLowerCase() === "text/markdown" &&
            !parameters.some((parameter) => /^\s*q\s*=\s*0(?:\.0*)?\s*$/i.test(parameter))
          );
        })
    ) {
      return homepageMarkdown(markdownUrl, request.method);
    }
    const response = NextResponse.next();
    response.headers.set("Link", HOMEPAGE_DISCOVERY_LINKS);
    if (markdownUrl) {
      response.headers.append(
        "Link",
        [
          '</.well-known/api-catalog>; rel="api-catalog"',
          '</llms.txt>; rel="describedby"; type="text/markdown"',
          '</auth.md>; rel="describedby"; type="text/markdown"',
        ].join(", "),
      );
      response.headers.append("Vary", "Accept");
    }
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
    "/virtual-users/:path*",
    "/agents/:path*",
    "/capabilities/:path*",
    "/chat/:path*",
    "/chats/:path*",
    "/dashboard/:path*",
    "/durable/:path*",
    "/evals/:path*",
    "/harnesses/:path*",
    "/models/:path*",
    "/orgs/:path*",
    "/sessions/:path*",
    "/settings/:path*",
    "/skills/:path*",
    "/memory/:path*",
  ],
};
