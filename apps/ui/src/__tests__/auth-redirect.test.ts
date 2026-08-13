import {
  buildLoginHref,
  buildSignupHref,
  consumeSignupEmail,
  consumeReturnTo,
  getLoginRedirectPath,
  getPostAuthTarget,
  isFullPageLoginRedirect,
  navigateToLogin,
  persistReturnTo,
  persistSignupEmail,
  RETURN_TO_STORAGE_KEY,
  SIGNUP_EMAIL_STORAGE_KEY,
  sanitizeReturnTo,
} from "@/lib/auth-redirect";

describe("getLoginRedirectPath", () => {
  it("preserves the current path and query in return_to", () => {
    expect(getLoginRedirectPath("/settings/providers", new URLSearchParams("tab=models"))).toBe(
      "/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels",
    );
  });

  it("omits return_to for the default landing page", () => {
    expect(getLoginRedirectPath("/chats", new URLSearchParams())).toBe("/login");
  });

  it("preserves dashboard queries because they are not the default landing page", () => {
    expect(getLoginRedirectPath("/dashboard", new URLSearchParams("tab=recent"))).toBe(
      "/login?return_to=%2Fdashboard%3Ftab%3Drecent",
    );
  });

  it("uses the configured trusted login origin", () => {
    expect(
      getLoginRedirectPath(
        "/settings/providers",
        new URLSearchParams("tab=models"),
        "https://id.example.com",
      ),
    ).toBe("https://id.example.com/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels");
  });

  it("round-trips an OAuth authorize continuation through the configured origin", () => {
    expect(
      getLoginRedirectPath(
        "/oauth/authorize",
        new URLSearchParams("client_id=test&response_type=code"),
        "https://id.example.com/",
      ),
    ).toBe(
      "https://id.example.com/login?return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dtest%26response_type%3Dcode",
    );
  });

  it("rejects a configured destination that is not an origin", () => {
    expect(() =>
      getLoginRedirectPath("/settings", null, "https://id.example.com/login?next=evil"),
    ).toThrow("Login origin must be an HTTP(S) origin");
  });
});

describe("isFullPageLoginRedirect", () => {
  it("uses client routing only for same-origin relative login paths", () => {
    expect(isFullPageLoginRedirect("/login?return_to=%2Fsettings")).toBe(false);
    expect(isFullPageLoginRedirect("https://id.example.com/login?return_to=%2Fsettings")).toBe(
      true,
    );
  });

  it("never sends an absolute login URL through the client router", () => {
    const replace = jest.fn();
    const assign = jest.fn();
    const target = "https://id.example.com/login?return_to=%2Fsettings";

    navigateToLogin(target, replace, assign);

    expect(assign).toHaveBeenCalledWith(target);
    expect(replace).not.toHaveBeenCalled();
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
  it("includes sanitized return_to when provided", () => {
    expect(buildSignupHref("/invite/abc123")).toBe("/signup?return_to=%2Finvite%2Fabc123");
  });

  it("omits unsafe return_to values", () => {
    expect(buildSignupHref("https://evil.com")).toBe("/signup");
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

describe("getPostAuthTarget", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it("uses the URL return_to and clears any stored one-shot redirect", () => {
    persistReturnTo("/invite/token123");

    expect(getPostAuthTarget("/invite/token123")).toBe("/invite/token123");
    expect(sessionStorage.getItem(RETURN_TO_STORAGE_KEY)).toBeNull();
  });

  it("falls back to the stored return_to when the URL has none", () => {
    persistReturnTo("/settings/providers");

    expect(getPostAuthTarget(null)).toBe("/settings/providers");
    expect(sessionStorage.getItem(RETURN_TO_STORAGE_KEY)).toBeNull();
  });

  it("falls back to the landing surface when no safe target exists", () => {
    expect(getPostAuthTarget("https://evil.com")).toBe("/chats");
  });
});

describe("signup email session storage", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it("persists and consumes a typed signup email without a URL parameter", () => {
    persistSignupEmail(" new@example.com ");
    expect(sessionStorage.getItem(SIGNUP_EMAIL_STORAGE_KEY)).toBe("new@example.com");
    expect(consumeSignupEmail()).toBe("new@example.com");
    expect(sessionStorage.getItem(SIGNUP_EMAIL_STORAGE_KEY)).toBeNull();
  });

  it("ignores blank signup email values", () => {
    persistSignupEmail("   ");
    expect(sessionStorage.getItem(SIGNUP_EMAIL_STORAGE_KEY)).toBeNull();
  });
});
