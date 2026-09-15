/**
 * Every API request names the org it belongs to.
 *
 * The `everruns_org` cookie is synced by a separate request that lands later
 * than the client's own org commit, so a request issued in between used to be
 * answered in the previous org while its result was cached under the new one.
 * `X-Org-Id` closes that window (EVE-984 made the server honour it).
 */
import { api } from "@/lib/api/client";
import { setActiveOrgId, withOrgHeader } from "@/lib/api/active-org";

const mockFetch = jest.fn();

beforeEach(() => {
  jest.clearAllMocks();
  setActiveOrgId(null);
  global.fetch = mockFetch as unknown as typeof fetch;
  mockFetch.mockResolvedValue({
    ok: true,
    status: 200,
    text: async () => JSON.stringify({ data: [] }),
  });
});

function sentHeaders(): Record<string, string> {
  return mockFetch.mock.calls[0][1].headers as Record<string, string>;
}

test("sends the active org on every request", async () => {
  setActiveOrgId("org_test1");

  await api.get("/v1/sessions");

  expect(sentHeaders()["X-Org-Id"]).toBe("org_test1");
});

test("falls back to the cookie when no org is active yet", async () => {
  await api.get("/v1/auth/me");

  expect(sentHeaders()["X-Org-Id"]).toBeUndefined();
});

test("follows the org across a switch, without waiting for the cookie", async () => {
  setActiveOrgId("org_personal");
  await api.get("/v1/sessions");
  setActiveOrgId("org_test1");
  await api.get("/v1/sessions");

  expect((mockFetch.mock.calls[0][1].headers as Record<string, string>)["X-Org-Id"]).toBe(
    "org_personal",
  );
  expect((mockFetch.mock.calls[1][1].headers as Record<string, string>)["X-Org-Id"]).toBe(
    "org_test1",
  );
});

test("an explicit header from the caller wins", () => {
  setActiveOrgId("org_test1");

  expect(withOrgHeader({ "X-Org-Id": "org_other" })["X-Org-Id"]).toBe("org_other");
});
