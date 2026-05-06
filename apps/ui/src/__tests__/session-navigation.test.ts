import { buildSessionNavigation } from "@/components/session/session-header";

describe("buildSessionNavigation", () => {
  it("includes the context report in advanced session navigation", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(),
    });

    expect(items.map((item) => item.key)).toContain("context");
    expect(items.find((item) => item.key === "context")?.href).toBe(
      "/sessions/session_123/context",
    );
  });
});
