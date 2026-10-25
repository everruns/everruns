import { isPageQuery, pageLocation, summarizePageData } from "@/lib/chat-page-view";

it("keeps useful page identity across resource and settings routes", () => {
  expect(pageLocation("/harnesses/harness_123?tab=preview")).toMatchObject({
    entity_type: "harness",
    entity_id: "harness_123",
    label: "Harnesses",
  });
  expect(pageLocation("/agents/new").entity_id).toBeUndefined();
  expect(pageLocation("/settings/profile", "My profile").label).toBe("My profile");
  expect(pageLocation("/settings/profile").label).toBe("Account · Settings");
  expect(pageLocation("/settings/billing").label).toBe("billing · Settings");
});

it("identifies nested resources without treating a new form as its parent", () => {
  expect(pageLocation("/models/providers/provider_0123456789abcdef0123456789abcdef")).toMatchObject(
    { entity_type: "provider", entity_id: "provider_0123456789abcdef0123456789abcdef" },
  );
  expect(pageLocation("/agents/agent_1/channels/new").entity_id).toBeUndefined();
  expect(
    pageLocation("/sandboxes/templates/template_0123456789abcdef0123456789abcdef").entity_type,
  ).toBe("sandbox_template");
});

it("bounds a large loaded page and still includes nested infinite-list identities", () => {
  const budget = { nodes: 0, characters: 0, truncated: false };
  const summary = summarizePageData(
    { pages: [{ data: [{ id: "visible-row", description: "x".repeat(20000) }] }] },
    0,
    budget,
  );
  expect(JSON.stringify(summary)).toContain("visible-row");
  expect(budget.truncated).toBe(true);
  expect(budget.characters).toBeLessThanOrEqual(16384);
});

it("reads only the current resource and organization, never credential or log queries", () => {
  const page = pageLocation("/agents/agent_1");
  expect(isPageQuery(["agent", "agent_1", "org_1"], page, "org_1")).toBe(true);
  expect(isPageQuery(["agent", "agent_2", "org_1"], page, "org_1")).toBe(false);
  expect(isPageQuery(["agent", "agent_1", "org_2"], page, "org_1")).toBe(false);
  expect(isPageQuery(["agent", "agent_1", "credentials", "org_1"], page, "org_1")).toBe(false);
  expect(isPageQuery(["session", "org_1", "agent_1", "context-report"], page, "org_1")).toBe(false);
  expect(
    isPageQuery(["agents", { search: "org_1" }, "org_2"], pageLocation("/agents"), "org_1"),
  ).toBe(false);
});

it("projects bounded non-secret summaries rather than serializing cached responses", () => {
  const summary = summarizePageData({
    name: "Example",
    api_key: "secret",
    config: { token: "secret" },
    messages: ["private"],
    items: [{ id: "one", name: "Visible", credentials: "secret" }],
  });
  expect(summary).toEqual({ name: "Example", items: [{ id: "one", name: "Visible" }] });
  expect(summarizePageData(Array.from({ length: 90 }, () => ({ name: "Row" })))).toHaveLength(50);
});
