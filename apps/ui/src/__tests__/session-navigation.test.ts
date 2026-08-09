import { buildSessionNavigation } from "@/components/session/session-header";

describe("buildSessionNavigation", () => {
  // A session is a recording (EVE-854): five tabs, no more, and the ones that
  // depend on a capability stay gated on the session's features.
  it("always offers timeline, events and cost", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(),
    });

    expect(items.map((item) => item.key)).toEqual(["timeline", "events", "cost"]);
    expect(items.find((item) => item.key === "timeline")?.href).toBe(
      "/sessions/session_123/timeline",
    );
  });

  it("adds workspace and work when the session has those capabilities", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "leased_resources"]),
    });

    expect(items.map((item) => item.key)).toEqual(["timeline", "work", "events", "files", "cost"]);
    expect(items.find((item) => item.key === "files")?.label).toBe("Workspace");
  });

  it("never offers a tab the recording cannot show read-only", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "secrets", "key_value", "schedules", "leased_resources"]),
    });

    const keys = items.map((item) => item.key);
    for (const retired of ["chat", "trajectory", "storage", "resources", "schedules", "context"]) {
      expect(keys).not.toContain(retired);
    }
  });
});
