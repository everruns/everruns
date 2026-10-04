import type { Session } from "@/lib/api/types";
import {
  groupPlaygroundSessions,
  nameInitials,
  playgroundDayLabel,
  playgroundPreview,
  playgroundRowTime,
  playgroundStarter,
  playgroundStatus,
  principalDisplayName,
} from "@/lib/playground-list";

const now = new Date(2026, 9, 3, 17, 0);

function session(overrides: Partial<Session> = {}): Session {
  return {
    id: "session_test",
    source: "playground",
    title: "Support test",
    preview: "Hello",
    agent_id: "agent_support",
    harness_id: "harness_base",
    playground_user_id: "identity_customer",
    activity: "idle",
    updated_at: "2026-10-03T16:41:00",
    tags: [],
    organization_id: "org_test",
    owner_principal_id: "principal_test",
    model_id: null,
    status: "started",
    created_at: "2026-10-03T16:41:00",
    started_at: null,
    finished_at: null,
    ...overrides,
  };
}

test("day labels and row times follow today, yesterday, and earlier", () => {
  expect(playgroundDayLabel("2026-10-03T16:41:00", now)).toBe("Today");
  expect(playgroundDayLabel("2026-10-02T01:13:00", now)).toBe("Yesterday");
  expect(playgroundDayLabel("2026-10-01T09:05:00", now)).toBe("Earlier");
  expect(playgroundRowTime("2026-10-02T01:13:00", now)).toBe("Yesterday");
  expect(playgroundRowTime("2026-10-01T09:05:00", now)).toBe(
    new Date(2026, 9, 1, 9, 5).toLocaleDateString(undefined, { month: "short", day: "numeric" }),
  );
  expect(playgroundRowTime("2026-10-03T16:41:00", now)).toMatch(/4:41/);
});

test("preview prefers the latest assistant text and empty chats stay empty", () => {
  expect(playgroundPreview(session({ output_preview: "Refund issued", preview: "Hello" }))).toBe(
    "Refund issued",
  );
  expect(playgroundPreview(session({ preview: "Hello", output_preview: "  " }))).toBe("Hello");
  expect(playgroundPreview(session({ preview: null, output_preview: null }))).toBe(
    "No messages yet",
  );
  expect(playgroundStatus(session({ preview: null, output_preview: null, event_count: 0 }))).toBe(
    "empty",
  );
  expect(playgroundStatus(session({ activity: "running", event_count: 3 }))).toBe("running");
  expect(playgroundStatus(session({ activity: "failed" }))).toBe("failed");
});

test("starter initials come from the effective owner, not the virtual user", () => {
  expect(principalDisplayName({ id: "p", kind: "user", metadata: { name: "  " } })).toBeNull();
  expect(nameInitials("Mykola Chaliy")).toBe("MC");
  expect(nameInitials("Alex")).toBe("AL");
  expect(
    playgroundStarter(
      session({
        effective_owner: { id: "principal_human", kind: "user", metadata: { name: "Jana Kovac" } },
        owner: { id: "principal_subject", kind: "virtual_user", metadata: { name: "Priya Shah" } },
      }),
    ),
  ).toEqual({ name: "Jana Kovac", initials: "JK" });
});

test("grouping keeps newest-activity order and gathers the same agent", () => {
  const rows = [
    { updatedAt: "2026-10-03T16:00:00", agentLabel: "Billing triage" },
    { updatedAt: "2026-10-03T15:00:00", agentLabel: "Support assistant" },
    { updatedAt: "2026-10-03T14:00:00", agentLabel: "Billing triage" },
    { updatedAt: "2026-10-02T11:00:00", agentLabel: "Billing triage" },
  ];
  expect(groupPlaygroundSessions(rows, "day", now).map((group) => group.label)).toEqual([
    "Today",
    "Yesterday",
  ]);
  expect(
    groupPlaygroundSessions(rows, "agent", now).map((group) => [group.label, group.rows.length]),
  ).toEqual([
    ["Billing triage", 3],
    ["Support assistant", 1],
  ]);
  expect(groupPlaygroundSessions(rows, "none", now)).toEqual([{ label: "", rows }]);
  expect(groupPlaygroundSessions([], "day", now)).toEqual([]);
});
