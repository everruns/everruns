import { defaultNavigationSections, navigationGroupForPath } from "@/lib/navigation";

// EVE-869: a page's group is decided in exactly one place — the sidebar section
// table. These pin the derivation the breadcrumb depends on.
describe("navigationGroupForPath", () => {
  it("names the group that owns a list route", () => {
    expect(navigationGroupForPath("/sessions")).toBe("Operational");
    expect(navigationGroupForPath("/agents")).toBe("Building");
    expect(navigationGroupForPath("/models")).toBe("Registries");
    expect(navigationGroupForPath("/evals")).toBe("Quality");
  });

  it("carries the group down to detail and sub-routes", () => {
    expect(navigationGroupForPath("/sessions/ses-abc123")).toBe("Operational");
    expect(navigationGroupForPath("/agents/agent-1/edit")).toBe("Building");
    expect(navigationGroupForPath("/agents/all")).toBe("Building");
  });

  it("matches on segment boundaries, not string prefixes", () => {
    // `/agent-identities` starts with `/agent` but is its own route; a naive
    // prefix match would resolve it through `/agents`.
    expect(navigationGroupForPath("/agent-identities")).toBe("Building");
    expect(navigationGroupForPath("/agent-identities/id-1")).toBe("Building");
  });

  it("prefers the longest matching route", () => {
    // Both `/durable` and `/durable/workers` match; the more specific wins.
    expect(navigationGroupForPath("/durable/workers")).toBe("Durable Execution");
  });

  it("gives no group to pages outside a labelled section", () => {
    // Chats and Settings have no group header in the sidebar, so their pages
    // take no prefix.
    expect(navigationGroupForPath("/chats")).toBeUndefined();
    expect(navigationGroupForPath("/chats/thread-1")).toBeUndefined();
    expect(navigationGroupForPath("/settings/organization")).toBeUndefined();
    expect(navigationGroupForPath("/not-a-route")).toBeUndefined();
  });

  it("follows the section table, so regrouping a route needs no page change", () => {
    const regrouped = defaultNavigationSections.map((section) =>
      section.label === "Building" ? { ...section, label: "Workshop" } : section,
    );
    expect(navigationGroupForPath("/agents/agent-1", regrouped)).toBe("Workshop");
  });
});
