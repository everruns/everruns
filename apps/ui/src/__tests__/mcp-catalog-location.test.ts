import nextConfig from "../../next.config";
import { defaultNavigationSections, visibleNavigationSections } from "@/lib/navigation";
import { settingsNavigationSections } from "@/lib/settings-navigation";
import type { FeatureFlags } from "@/lib/api/types";

// knowledge/integrations/user-mcp-servers.md, Plan step 8: the org MCP catalog
// lives in Settings > Organization, visible only to people who manage it.
describe("MCP catalog location", () => {
  const flags = {} as FeatureFlags;
  const hasRole = () => true;

  it("redirects the old catalog list and detail URLs to Settings", async () => {
    const redirects = (await nextConfig.redirects?.()) ?? [];
    const old = redirects.filter((rule) => rule.source.startsWith("/mcp-servers"));
    expect(old.map((rule) => rule.source).sort()).toEqual(["/mcp-servers", "/mcp-servers/:path*"]);
    for (const rule of old) {
      expect(rule.destination).toBe("/settings/mcp-catalog");
      expect(rule.permanent).toBe(true);
    }
  });

  it("removes the catalog from the main navigation for everyone", () => {
    const items = visibleNavigationSections(
      defaultNavigationSections,
      flags,
      hasRole,
      true,
      () => true,
    ).flatMap((section) => section.items);
    expect(items.some((item) => item.href.startsWith("/mcp-servers"))).toBe(false);
    expect(items.some((item) => item.name === "MCP")).toBe(false);
  });

  it("lists MCP catalog under Settings > Organization for people who manage it", () => {
    const sections = visibleNavigationSections(
      settingsNavigationSections,
      flags,
      hasRole,
      false,
      (policy) => policy === "mcp_server.manage",
    );
    const organization = sections.find((section) => section.label === "Organization");
    expect(organization?.items.find((item) => item.name === "MCP catalog")?.href).toBe(
      "/settings/mcp-catalog",
    );
  });

  it("hides the Settings entry from people who cannot manage the catalog", () => {
    const visible = (can?: (policy: string) => boolean) =>
      visibleNavigationSections(settingsNavigationSections, flags, hasRole, false, can)
        .flatMap((section) => section.items)
        .some((item) => item.href === "/settings/mcp-catalog");
    expect(visible((policy) => policy === "mcp_server.view")).toBe(false);
    expect(visible(() => false)).toBe(false);
    // No policy checker at all fails closed.
    expect(visible()).toBe(false);
  });
});
