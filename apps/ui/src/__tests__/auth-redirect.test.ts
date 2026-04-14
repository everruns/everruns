import { getLoginRedirectPath } from "@/lib/auth-redirect";

describe("getLoginRedirectPath", () => {
  it("preserves the current path and query in return_to", () => {
    expect(getLoginRedirectPath("/settings/providers", new URLSearchParams("tab=models"))).toBe(
      "/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels",
    );
  });

  it("omits return_to for the default dashboard landing page", () => {
    expect(getLoginRedirectPath("/dashboard", new URLSearchParams())).toBe("/login");
  });

  it("preserves dashboard queries because they are not the default landing page", () => {
    expect(getLoginRedirectPath("/dashboard", new URLSearchParams("tab=recent"))).toBe(
      "/login?return_to=%2Fdashboard%3Ftab%3Drecent",
    );
  });
});
