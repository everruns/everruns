/** @jest-environment node */

import { config, proxy } from "@/proxy";

describe("auth proxy", () => {
  const previousAuthMode = process.env.AUTH_MODE;
  const previousLoginOrigin = process.env.AUTH_LOGIN_ORIGIN;
  const previousMarkdownUrl = process.env.UI_HOMEPAGE_MARKDOWN_URL;

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
    if (previousMarkdownUrl === undefined) {
      delete process.env.UI_HOMEPAGE_MARKDOWN_URL;
    } else {
      process.env.UI_HOMEPAGE_MARKDOWN_URL = previousMarkdownUrl;
    }
    jest.restoreAllMocks();
  });

  it("redirects protected routes to the configured external login origin", async () => {
    process.env.AUTH_MODE = "full";
    process.env.AUTH_LOGIN_ORIGIN = "https://id.example.com";
    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("https://app.example.com/oauth/authorize?client_id=test"),
      url: "https://app.example.com/oauth/authorize?client_id=test",
    } as unknown as Parameters<typeof proxy>[0];

    const response = await proxy(request);

    expect(response.headers.get("location")).toBe(
      "https://id.example.com/login?return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dtest",
    );
  });

  it("redirects protected routes to login when the access token is missing", async () => {
    process.env.AUTH_MODE = "full";

    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as unknown as Parameters<typeof proxy>[0];

    const response = await proxy(request);

    expect(response.status).toBe(307);
    expect(response.headers.get("location")).toBe(
      "http://localhost/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels",
    );
  });

  it.each(["/chat", "/settings/providers?tab=models"])(
    "allows %s through when AUTH_MODE=none and cookie is missing",
    async (path) => {
      process.env.AUTH_MODE = "none";

      const request = {
        cookies: { has: () => false },
        nextUrl: new URL(`http://localhost${path}`),
        url: `http://localhost${path}`,
      } as unknown as Parameters<typeof proxy>[0];

      const response = await proxy(request);

      expect(response.headers.get("x-middleware-next")).toBe("1");
      expect(response.headers.get("location")).toBeNull();
    },
  );

  it("allows protected routes through when the access token cookie exists", async () => {
    const request = {
      cookies: { has: (name: string) => name === "access_token" },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as unknown as Parameters<typeof proxy>[0];

    const response = await proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
  });

  it("allows protected routes through when only the refresh token cookie exists", async () => {
    process.env.AUTH_MODE = "full";

    const request = {
      cookies: { has: (name: string) => name === "refresh_token" },
      nextUrl: new URL("http://localhost/settings/providers?tab=models"),
      url: "http://localhost/settings/providers?tab=models",
    } as unknown as Parameters<typeof proxy>[0];

    const response = await proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
  });

  it("adds agent discovery Link headers to the homepage", async () => {
    delete process.env.UI_HOMEPAGE_MARKDOWN_URL;
    const request = {
      cookies: { has: () => false },
      nextUrl: new URL("http://localhost/"),
      url: "http://localhost/",
    } as unknown as Parameters<typeof proxy>[0];

    const response = await proxy(request);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("location")).toBeNull();
    expect(response.headers.get("link")).toBe(
      '</api-doc/openapi.json>; rel="service-desc"; type="application/json", <https://everruns.com/docs/api/>; rel="service-doc"; type="text/html"',
    );
  });

  it.each(["GET", "HEAD"])("serves the public discovery document for %s", async (method) => {
    const origin = "http://everruns-server:9000/llms.txt";
    process.env.UI_HOMEPAGE_MARKDOWN_URL = origin;
    const fetchDocument = jest.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(method === "HEAD" ? null : "# Everruns", {
        headers: { "Content-Type": "text/markdown" },
      }),
    );
    const response = await proxy({
      nextUrl: new URL("https://everruns.example.com/"),
      method,
      headers: new Headers({
        Accept: "text/markdown",
        Authorization: "Bearer private",
      }),
    } as Parameters<typeof proxy>[0]);

    expect(fetchDocument).toHaveBeenCalledWith(origin, {
      method,
      redirect: "error",
      signal: expect.any(AbortSignal),
    });
    expect(response.headers.get("content-type")).toBe("text/markdown");
    expect(response.headers.get("vary")).toBe("Accept");
    expect(await response.text()).toBe(method === "HEAD" ? "" : "# Everruns");
  });

  it("keeps the browser homepage when Markdown is unacceptable", async () => {
    process.env.UI_HOMEPAGE_MARKDOWN_URL = "http://everruns-server:9000/llms.txt";
    const response = await proxy({
      nextUrl: new URL("https://everruns.example.com/"),
      method: "GET",
      headers: new Headers({ Accept: "text/html, text/markdown;q=0" }),
    } as Parameters<typeof proxy>[0]);

    expect(response.headers.get("x-middleware-next")).toBe("1");
    expect(response.headers.get("vary")).toBe("Accept");
    expect(response.headers.get("link")).toContain("</llms.txt>");
  });

  it("protects the main application routes", () => {
    expect(config.matcher).toEqual([
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
    ]);
  });
});
