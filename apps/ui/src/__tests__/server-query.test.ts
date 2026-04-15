import { getRequestOrigin, resolveCurrentOrgId } from "@/lib/server-query";
import type { OrganizationMembership } from "@/lib/api/types";

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

describe("server-query", () => {
  it("uses forwarded host and proto when present", () => {
    const headers = new Headers({
      host: "localhost:9100",
      "x-forwarded-host": "app.everruns.com",
      "x-forwarded-proto": "https",
    });

    expect(getRequestOrigin(headers)).toBe("https://app.everruns.com");
  });

  it("falls back to host header when forwarded headers are missing", () => {
    const headers = new Headers({
      host: "localhost:9100",
    });

    expect(getRequestOrigin(headers)).toBe("http://localhost:9100");
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
