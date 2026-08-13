/** @jest-environment node */

import { config, proxy } from "@/proxy";

describe("auth proxy", () => {
  const previousAuthMode = process.env.AUTH_MODE;
  const previousLoginOrigin = process.env.AUTH_LOGIN_ORIGIN;

  afterEach(() => {
    if (previousAuthMode === undefined) {
      delete process.env.AUTH_MODE;
    } else {
      process.env.AUTH_MODE = previousAuthMode;
    }
    if (previousLoginOrigin === undefined) {
      delete process.env.AUTH_LOGIN_ORIGIN;
    } else {
      process.env.AUTH_LOGIN_ORIGIN = previousLoginOrigin;
    }
  });

  it("redirects protected routes to the configured external login origin", () => {
    process.env.AUTH_MODE = "full";
    process.env.AUTH_LOGIN_ORIGIN = "https://id.example.com";
    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("https://app.example.com/oauth/authorize?client_id=test"),
      url: "https://app.example.com/oauth/authorize?client_id=test",
    } as unknown as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.headers.get("location")).toBe(
      "https://id.example.com/login?return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dtest",
    );
  });

  it("redirects protected routes to login when the access token is missing", () => {
    process.env.AUTH_MODE = "full";

    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as unknown as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.status).toBe(307);
    expect(response.headers.get("location")).toBe(
      "http://localhost/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels",
    );
  });

  it.each(["/chat", "/settings/providers?tab=models"])(
    "allows %s through when AUTH_MODE=none and cookie is missing",
    (path) => {
      process.env.AUTH_MODE = "none";

      const request = {
        cookies: { has: () => false },
        nextUrl: new URL(`http://localhost${path}`),
        url: `http://localhost${path}`,
      } as unknown as Parameters<typeof proxy>[0];

      const response = proxy(request);

      expect(response.headers.get("x-middleware-next")).toBe("1");
      expect(response.headers.get("location")).toBeNull();
    },
  );

  it("allows protected routes through when the access token cookie exists", () => {
    const request = {
      cookies: { has: (name: string) => name === "access_token" },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as unknown as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
  });

  it("allows protected routes through when only the refresh token cookie exists", () => {
    process.env.AUTH_MODE = "full";

    const request = {
      cookies: { has: (name: string) => name === "refresh_token" },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as unknown as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
  });

  it("adds agent discovery Link headers to the homepage", () => {
    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("http://localhost/"),
      url: "http://localhost/",
    } as unknown as Parameters<typeof proxy>[0];

    const response = proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
    expect(response.headers.get("link")).toBe(
      '</api-doc/openapi.json>; rel="service-desc"; type="application/json", <https://docs.everruns.com/api/>; rel="service-doc"; type="text/html"',
    );
  });

  it("protects the main application routes", () => {
    expect(config.matcher).toEqual([
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
    ]);
  });
});
