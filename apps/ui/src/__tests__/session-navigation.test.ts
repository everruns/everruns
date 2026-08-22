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

  // EVE-868: the tab bar is a map of the recording. Counts come from the
  // session payload the page already fetched, so badging costs no extra fetch.
  it("badges work, events and workspace with what is behind them", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "leased_resources"]),
      taskCount: 3,
      eventCount: 42,
      fileCount: 6,
    });

    const badgeFor = (key: string) => items.find((item) => item.key === key)?.badge;
    expect(badgeFor("work")).toBe("3");
    expect(badgeFor("events")).toBe("42");
    expect(badgeFor("files")).toBe("6");
  });

  it("leaves timeline, transcript and cost unbadged", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "leased_resources"]),
      taskCount: 3,
      eventCount: 42,
      fileCount: 6,
    });

    for (const key of ["transcript", "timeline", "cost"]) {
      expect(items.find((item) => item.key === key)?.badge).toBeUndefined();
    }
  });

  it("renders an empty tab as no badge rather than a zero", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "leased_resources"]),
      taskCount: 0,
      eventCount: 0,
      fileCount: 0,
    });

    for (const key of ["work", "events", "files"]) {
      expect(items.find((item) => item.key === key)?.badge).toBeUndefined();
    }
  });

  it("omits badges entirely when the payload carries no counts", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(["file_system", "leased_resources"]),
    });

    expect(items.every((item) => item.badge === undefined)).toBe(true);
  });

  it("compacts a long recording so the tab bar does not stretch", () => {
    const items = buildSessionNavigation({
      basePath: "/sessions/session_123",
      features: new Set(),
      eventCount: 12_400,
    });

    expect(items.find((item) => item.key === "events")?.badge).toBe("12.4K");
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
