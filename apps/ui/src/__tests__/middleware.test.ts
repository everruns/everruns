/** @jest-environment node */

import { config, proxy } from "@/proxy";

describe("auth proxy", () => {
  it("redirects protected routes to login when the access token is missing", () => {
    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.status).toBe(307);
    expect(response.headers.get("location")).toBe(
      "http://localhost/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels",
    );
  });

  it("allows protected routes through when the access token cookie exists", () => {
    const request = {
      cookies: { has: (name: string) => name === "access_token" },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
  });

  it("protects the main application routes", () => {
    expect(config.matcher).toEqual([
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
    ]);
  });
});
