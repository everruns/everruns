import {
  defaultBuildNavigation,
  defaultNavigationSections,
  navigationGroupForPath,
} from "@/lib/navigation";

// EVE-869: a page's group is decided in exactly one place — the sidebar section
// table. These pin the derivation the breadcrumb depends on.
describe("navigationGroupForPath", () => {
  it("carries the group down to detail and sub-routes", () => {
    expect(navigationGroupForPath("/sessions/ses-abc123")).toBe("Observability");
    expect(navigationGroupForPath("/agents/agent-1/edit")).toBe("Build");
    expect(navigationGroupForPath("/agents/all")).toBe("Build");
  });

  it("matches on segment boundaries, not string prefixes", () => {
    // `/virtual-users` starts with `/agent` but is its own route; a naive
    // prefix match would resolve it through `/agents`.
    expect(navigationGroupForPath("/virtual-users")).toBe("Blocks");
    expect(navigationGroupForPath("/virtual-users/id-1")).toBe("Blocks");
  });

  it("prefers the longest matching route", () => {
    // Both `/durable` and `/durable/workers` match; the more specific wins.
    expect(navigationGroupForPath("/durable/workers")).toBe("System");
    expect(navigationGroupForPath("/system/health")).toBe("System");
  });

  it("gives no group to pages outside a labelled section", () => {
    // Chats, Overview and Settings have no group header in the sidebar, so
    // their pages take no prefix.
    expect(navigationGroupForPath("/home")).toBeUndefined();
    expect(navigationGroupForPath("/chats")).toBeUndefined();
    expect(navigationGroupForPath("/chats/thread-1")).toBeUndefined();
    expect(navigationGroupForPath("/settings/organization")).toBeUndefined();
    expect(navigationGroupForPath("/not-a-route")).toBeUndefined();
  });

  it("follows the section table, so regrouping a route needs no page change", () => {
    const regrouped = defaultNavigationSections.map((section) =>
      section.label === "Build" ? { ...section, label: "Workshop" } : section,
    );
    expect(navigationGroupForPath("/agents/agent-1", regrouped)).toBe("Workshop");
  });

  it("groups destinations as Build, Observability, Blocks and Quality", () => {
    const groups = Object.fromEntries(
      defaultNavigationSections
        .filter((section) => section.label)
        .map((section) => [section.label, section.items.map((item) => item.name)]),
    );
    expect(Object.keys(groups).slice(0, 4)).toEqual([
      "Build",
      "Observability",
      "Blocks",
      "Quality",
    ]);
    expect(groups.Build).toEqual(["Agents", "Playground", "Extractions"]);
    expect(groups.Observability).toEqual(["Sessions", "Rooms", "Monitoring"]);
    expect(groups.Blocks).toEqual([
      "Harnesses",
      "Capabilities",
      "Skills",
      "Knowledge",
      "Memory",
      "Virtual Users",
      "Models",
      "Web search",
      "Sandboxes",
    ]);
  });
});

describe("Build navigation", () => {
  it("puts adoption-gated Extractions last", () => {
    expect(defaultBuildNavigation.at(-1)).toMatchObject({
      name: "Extractions",
      href: "/extractions",
      flag: "extractions",
    });
  });
});
