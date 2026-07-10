import {
  buildLoginHref,
  buildSignupHref,
  consumeReturnTo,
  getLoginRedirectPath,
  persistReturnTo,
  RETURN_TO_STORAGE_KEY,
  sanitizeReturnTo,
} from "@/lib/auth-redirect";

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

describe("sanitizeReturnTo", () => {
  it("accepts frontend-relative paths", () => {
    expect(sanitizeReturnTo("/dashboard")).toBe("/dashboard");
    expect(sanitizeReturnTo("/settings/providers?tab=models")).toBe(
      "/settings/providers?tab=models",
    );
  });

  it("accepts backend-facing paths used for full-page navigation", () => {
    expect(sanitizeReturnTo("/oauth/authorize?client_id=x")).toBe("/oauth/authorize?client_id=x");
    expect(sanitizeReturnTo("/api/v1/auth/cli/callback?state=abc")).toBe(
      "/api/v1/auth/cli/callback?state=abc",
    );
  });

  it("rejects absolute URLs", () => {
    expect(sanitizeReturnTo("https://evil.com/takeover")).toBeNull();
    expect(sanitizeReturnTo("http://evil.com/takeover")).toBeNull();
  });

  it("rejects protocol-relative URLs", () => {
    expect(sanitizeReturnTo("//evil.com/takeover")).toBeNull();
  });

  it("rejects backslash-prefixed paths (browser may normalize to //)", () => {
    expect(sanitizeReturnTo("/\\evil.com")).toBeNull();
  });

  it("rejects values that don't start with a slash", () => {
    expect(sanitizeReturnTo("dashboard")).toBeNull();
    expect(sanitizeReturnTo("evil.com")).toBeNull();
  });

  it("rejects empty / nullish values", () => {
    expect(sanitizeReturnTo(null)).toBeNull();
    expect(sanitizeReturnTo(undefined)).toBeNull();
    expect(sanitizeReturnTo("")).toBeNull();
  });
});

describe("buildSignupHref", () => {
  it("includes sanitized return_to and email when provided", () => {
    expect(buildSignupHref("/invite/abc123", "new@example.com")).toBe(
      "/signup?email=new%40example.com&return_to=%2Finvite%2Fabc123",
    );
  });

  it("omits unsafe return_to values", () => {
    expect(buildSignupHref("https://evil.com", "new@example.com")).toBe(
      "/signup?email=new%40example.com",
    );
  });
});

describe("buildLoginHref", () => {
  it("includes sanitized return_to when provided", () => {
    expect(buildLoginHref("/invite/abc123")).toBe("/login?return_to=%2Finvite%2Fabc123");
  });
});

describe("return_to session storage", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it("persists and consumes sanitized return_to values", () => {
    persistReturnTo("/invite/token123");
    expect(sessionStorage.getItem(RETURN_TO_STORAGE_KEY)).toBe("/invite/token123");
    expect(consumeReturnTo()).toBe("/invite/token123");
    expect(sessionStorage.getItem(RETURN_TO_STORAGE_KEY)).toBeNull();
  });

  it("ignores unsafe return_to values", () => {
    persistReturnTo("//evil.com");
    expect(sessionStorage.getItem(RETURN_TO_STORAGE_KEY)).toBeNull();
    expect(consumeReturnTo()).toBeNull();
  });
});
