import {
  buildLoginHref,
  buildSignupHref,
  consumeSignupEmail,
  consumeReturnTo,
  getLoginRedirectPath,
  getPostAuthTarget,
  isBackendNavigationPath,
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
    for (const origin of [
      "https://id.example.com/login?next=evil",
      "https://user:password@id.example.com",
      "https://id.example.com/#fragment",
      "ftp://id.example.com",
      "/login",
    ]) {
      expect(() => getLoginRedirectPath("/settings", null, origin)).toThrow();
    }
  });
});

describe("isFullPageLoginRedirect", () => {
  it("dispatches each login destination through exactly one navigation mechanism", () => {
    for (const [target, fullPage] of [
      ["https://id.example.com/login?return_to=%2Fsettings", true],
      ["HTTP://id.example.com/login", true],
      ["/login?return_to=%2Fsettings", false],
    ] as const) {
      const replace = jest.fn();
      const assign = jest.fn();
      navigateToLogin(target, replace, assign);
      expect(fullPage ? assign : replace).toHaveBeenCalledTimes(1);
      expect(fullPage ? assign : replace).toHaveBeenCalledWith(target);
      expect(fullPage ? replace : assign).not.toHaveBeenCalled();
    }
  });
});

describe("sanitizeReturnTo", () => {
  it("preserves safe relative paths and rejects absolute or ambiguous targets", () => {
    for (const path of [
      "/dashboard",
      "/settings/providers?tab=models",
      "/oauth/authorize?client_id=x",
      "/api/v1/auth/cli/callback?state=abc",
      "/settings/%2Fvalue#section",
    ]) {
      expect(sanitizeReturnTo(path)).toBe(path);
      expect(new URL(path, "https://app.example.test").origin).toBe("https://app.example.test");
    }
    for (const path of [
      null,
      undefined,
      "",
      "https://evil.com/takeover",
      "http://evil.com/takeover",
      "//evil.com/takeover",
      "/\\evil.com",
      "dashboard",
      "evil.com",
    ]) {
      expect(sanitizeReturnTo(path)).toBeNull();
    }
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
    expect(consumeReturnTo()).toBeNull();
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
    persistReturnTo("/settings/providers");

    expect(getPostAuthTarget("/invite/token123")).toBe("/invite/token123");
    expect(getPostAuthTarget(null)).toBe("/chats");
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
    expect(consumeSignupEmail()).toBe("");
    expect(sessionStorage.getItem(SIGNUP_EMAIL_STORAGE_KEY)).toBeNull();
  });

  it("ignores blank signup email values", () => {
    persistSignupEmail("   ");
    expect(sessionStorage.getItem(SIGNUP_EMAIL_STORAGE_KEY)).toBeNull();
  });
});

describe("browser URL normalization", () => {
  it("rejects paths that become external after browser control-character removal", () => {
    for (const path of ["/\n/evil.test", "/\t\\evil.test", "/\r/evil.test"]) {
      expect(new URL(path, "https://app.example.test").origin).toBe("https://evil.test");
      expect(sanitizeReturnTo(path)).toBeNull();
      expect(buildLoginHref(path)).toBe("/login");
    }
  });
});

describe("redirect boundary contracts", () => {
  beforeEach(() => sessionStorage.clear());

  it("preserves string search inputs and canonicalizes configured origins", () => {
    for (const search of ["?tab=models", "tab=models", new URLSearchParams("tab=models")]) {
      expect(getLoginRedirectPath("/chats", search, "HTTPS://ID.EXAMPLE.COM:443/")).toBe(
        "https://id.example.com/login?return_to=%2Fchats%3Ftab%3Dmodels",
      );
    }
    for (const search of [null, undefined, "", "?"]) {
      expect(getLoginRedirectPath("/chats", search)).toBe("/login");
    }
  });

  it("both auth links encode safe targets and omit unsafe targets", () => {
    for (const [build, base] of [
      [buildLoginHref, "/login"],
      [buildSignupHref, "/signup"],
    ] as const) {
      expect(build("/settings?tab=a&next=b#section")).toBe(
        `${base}?return_to=%2Fsettings%3Ftab%3Da%26next%3Db%23section`,
      );
      for (const bad of [
        null,
        undefined,
        "",
        "https://evil.test",
        "//evil.test",
        "/\n/evil.test",
      ]) {
        expect(build(bad)).toBe(base);
      }
    }
  });

  it("revalidates poisoned storage and consumes only the owned key", () => {
    sessionStorage.setItem("unrelated", "keep");
    for (const bad of ["https://evil.test", "/\n/evil.test", "//evil.test"]) {
      sessionStorage.setItem(RETURN_TO_STORAGE_KEY, bad);
      expect(consumeReturnTo()).toBeNull();
      expect(sessionStorage.getItem(RETURN_TO_STORAGE_KEY)).toBeNull();
    }
    expect(sessionStorage.getItem("unrelated")).toBe("keep");
    persistReturnTo("/settings");
    expect(getPostAuthTarget("https://evil.test")).toBe("/settings");
    expect(getPostAuthTarget(null)).toBe("/chats");
  });

  it("routes backend continuations without matching similar frontend paths", () => {
    for (const path of [
      "/oauth/authorize?client_id=x",
      "/api/v1/auth/cli/callback",
      "/v1/auth/cli/callback",
    ]) {
      expect(isBackendNavigationPath(path)).toBe(true);
    }
    for (const path of [
      "/oauth-settings",
      "/apiary",
      "/v10/chats",
      "/settings",
      "https://evil.test/api/",
    ]) {
      expect(isBackendNavigationPath(path)).toBe(false);
    }
  });
});
