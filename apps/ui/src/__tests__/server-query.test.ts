/**
 * @jest-environment node
 */
import { QueryClient } from "@tanstack/react-query";
import {
  getServerRequestContext,
  prefetchAuthBootstrap,
  resolveCurrentOrgId,
  resolveTrustedApiBaseUrl,
  serverGet,
} from "@/lib/server-query";
import type { OrganizationMembership } from "@/lib/api/types";

// A request with an attacker-chosen Host and a dummy auth cookie. `headers()`
// is mocked so the attack input is present even though the helper must not read it.
const ATTACKER_HOST = "attacker.example";
const INTERNAL_REDIRECT_TARGET = "http://169.254.169.254/latest/meta-data/";
const DUMMY_COOKIE = "access_token=dummy-access; everruns_org=org_second";

jest.mock("next/headers", () => ({
  headers: async () =>
    new Headers({
      host: "attacker.example",
      "x-forwarded-host": "attacker.example",
      "x-forwarded-proto": "https",
    }),
  cookies: async () => ({
    toString: (): string => "access_token=dummy-access; everruns_org=org_second",
    get: (name: string) => (name === "everruns_org" ? { value: "org_second" } : undefined),
  }),
}));

const DEFAULT_ORG: OrganizationMembership = {
  public_id: "org_00000000000000000000000000000001",
  name: "Default Org",
  role: "owner",
};

const SECOND_ORG: OrganizationMembership = {
  public_id: "org_second",
  name: "Second Org",
  role: "owner",
};

const ENV_KEYS = ["UI_SERVER_API_URL", "PUBLIC_APP_URL"] as const;

describe("server-query", () => {
  const savedEnv: Partial<Record<(typeof ENV_KEYS)[number], string>> = {};
  let fetchMock: jest.SpyInstance;

  beforeEach(() => {
    for (const key of ENV_KEYS) {
      savedEnv[key] = process.env[key];
      delete process.env[key];
    }
    fetchMock = jest.spyOn(globalThis, "fetch");
  });

  afterEach(() => {
    fetchMock.mockRestore();
    for (const key of ENV_KEYS) {
      if (savedEnv[key] === undefined) delete process.env[key];
      else process.env[key] = savedEnv[key];
    }
  });

  function requestedUrls(): string[] {
    return fetchMock.mock.calls.map(([input]) => String(input));
  }

  describe("trusted API origin (TM-WEB-019)", () => {
    it("fails closed without a configured origin: no outbound request, no cookie sent", async () => {
      const context = await getServerRequestContext();
      expect(context.apiBaseUrl).toBeNull();

      const result = await prefetchAuthBootstrap(new QueryClient(), context);

      expect(fetchMock).not.toHaveBeenCalled();
      expect(result).toEqual({ config: undefined, user: undefined, currentOrgId: null });
    });

    it("sends cookie-bearing requests only to the configured origin, never the request Host", async () => {
      process.env.UI_SERVER_API_URL = "http://server.internal:9000/api";
      fetchMock.mockImplementation(async () => Response.json({ organizations: [SECOND_ORG] }));

      const context = await getServerRequestContext();
      await prefetchAuthBootstrap(new QueryClient(), context);

      expect(requestedUrls()).toEqual([
        "http://server.internal:9000/api/v1/auth/config",
        "http://server.internal:9000/api/v1/auth/me",
      ]);
      for (const url of requestedUrls()) {
        expect(url).not.toContain(ATTACKER_HOST);
      }
      for (const [, init] of fetchMock.mock.calls) {
        expect(init).toMatchObject({ redirect: "manual", headers: { cookie: DUMMY_COOKIE } });
      }
    });

    it("refuses a redirect and never requests the redirect target", async () => {
      process.env.PUBLIC_APP_URL = "https://everruns.example.com";
      fetchMock.mockImplementation(
        async () =>
          new Response(null, { status: 302, headers: { location: INTERNAL_REDIRECT_TARGET } }),
      );

      const context = await getServerRequestContext();
      await expect(serverGet(context, "/v1/auth/me")).rejects.toThrow(/redirected/);

      expect(requestedUrls()).toEqual(["https://everruns.example.com/api/v1/auth/me"]);
      expect(fetchMock.mock.calls[0][1]).toMatchObject({ redirect: "manual" });
    });

    it("treats an opaque redirect response as failure", async () => {
      process.env.PUBLIC_APP_URL = "https://everruns.example.com";
      const opaque = new Response(null, { status: 200 });
      Object.defineProperty(opaque, "type", { value: "opaqueredirect" });
      fetchMock.mockResolvedValue(opaque);

      const context = await getServerRequestContext();
      await expect(serverGet(context, "/v1/auth/me")).rejects.toThrow(/redirected/);
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });
  });

  describe("resolveTrustedApiBaseUrl", () => {
    it("prefers UI_SERVER_API_URL and strips trailing slashes", () => {
      expect(
        resolveTrustedApiBaseUrl({
          UI_SERVER_API_URL: "http://server:9000/api/",
          PUBLIC_APP_URL: "https://everruns.example.com",
        }),
      ).toBe("http://server:9000/api");
    });

    it("derives the API base from PUBLIC_APP_URL", () => {
      expect(resolveTrustedApiBaseUrl({ PUBLIC_APP_URL: "https://everruns.example.com/" })).toBe(
        "https://everruns.example.com/api",
      );
    });

    it.each([
      [{}],
      [{ UI_SERVER_API_URL: "" }],
      [{ UI_SERVER_API_URL: "not a url" }],
      [{ UI_SERVER_API_URL: "file:///etc/passwd" }],
      [{ UI_SERVER_API_URL: "http://user:pass@server:9000/api" }],
      [{ UI_SERVER_API_URL: "http://server:9000/api?x=1" }],
      [{ PUBLIC_APP_URL: "https://everruns.example.com/app" }],
      [{ PUBLIC_APP_URL: "javascript:alert(1)" }],
    ])("rejects untrusted or malformed config %j", (env) => {
      expect(resolveTrustedApiBaseUrl(env)).toBeNull();
    });
  });

  it("prefers the cookie org when it is still in the membership list", () => {
    expect(resolveCurrentOrgId([DEFAULT_ORG, SECOND_ORG], SECOND_ORG.public_id)).toBe(
      SECOND_ORG.public_id,
    );
  });

  it("falls back to the default org when the cookie org is absent", () => {
    expect(resolveCurrentOrgId([DEFAULT_ORG, SECOND_ORG], "org_missing")).toBe(
      DEFAULT_ORG.public_id,
    );
  });
});
