import { legacyTabRedirect, tabFromPath, tabHref } from "@/lib/tab-routes";

const mockRedirect = jest.fn();
jest.mock("next/navigation", () => ({
  redirect: (path: string) => mockRedirect(path),
}));

const TABS = ["models", "providers", "defaults"] as const;

describe("tab routes", () => {
  beforeEach(() => mockRedirect.mockClear());

  it("puts the default tab at the base path and others beneath it", () => {
    expect(tabHref("/models", "models", "models")).toBe("/models");
    expect(tabHref("/models", "providers", "models")).toBe("/models/providers");
  });

  it("reads the tab from the path", () => {
    expect(tabFromPath("/models", "/models", TABS, "models")).toBe("models");
    expect(tabFromPath("/models/defaults", "/models", TABS, "models")).toBe("defaults");
    expect(tabFromPath("/models/unknown", "/models", TABS, "models")).toBe("models");
    expect(tabFromPath(null, "/models", TABS, "models")).toBe("models");
  });

  it("redirects a retired ?tab= link to the tab's path, keeping other params", () => {
    legacyTabRedirect("/models", { tab: "providers", provider: "p-1" }, TABS, "models");
    expect(mockRedirect).toHaveBeenCalledWith("/models/providers?provider=p-1");
  });

  it("sends an unknown or default ?tab= to the base path", () => {
    legacyTabRedirect("/models", { tab: "nope" }, TABS, "models");
    legacyTabRedirect("/models", { tab: "models", status: "all" }, TABS, "models");
    expect(mockRedirect.mock.calls).toEqual([["/models"], ["/models?status=all"]]);
  });

  it("leaves links without ?tab= alone", () => {
    legacyTabRedirect("/models", { provider: "p-1" }, TABS, "models");
    expect(mockRedirect).not.toHaveBeenCalled();
  });
});
