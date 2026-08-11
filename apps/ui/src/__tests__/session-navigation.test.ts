import { buildSessionNavigation } from "@/components/session/session-header";

describe("buildSessionNavigation", () => {
  // A session is a recording: Transcript, Timeline, Events, and Cost are
  // unconditional; the views that depend on capabilities stay feature-gated.
  it("always offers transcript first, followed by timeline, events and cost", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(),
    });

    expect(items.map((item) => item.key)).toEqual(["transcript", "timeline", "events", "cost"]);
    expect(items[0]).toMatchObject({
      label: "Transcript",
      href: "/sessions/session_123/transcript",
    });
    expect(items.find((item) => item.key === "timeline")?.href).toBe(
      "/sessions/session_123/timeline",
    );
  });

  it("adds workspace and work when the session has those capabilities", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "leased_resources"]),
    });

    expect(items.map((item) => item.key)).toEqual([
      "transcript",
      "timeline",
      "work",
      "events",
      "files",
      "cost",
    ]);
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
