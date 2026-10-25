import { visibleNavigationSections } from "@/lib/navigation";
import { settingsNavigationSections } from "@/lib/settings-navigation";
import type { FeatureFlags } from "@/lib/api/types";

// knowledge/integrations/user-mcp-servers.md, Plan step 8: the org MCP catalog
// lives in Settings > Organization, visible only to people who manage it.
describe("MCP catalog location", () => {
  const flags = {} as FeatureFlags;
  const hasRole = () => true;

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
