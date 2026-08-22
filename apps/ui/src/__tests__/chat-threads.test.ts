import { selectChatThreads, threadActivityAt, threadTitle } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";

function session(overrides: Partial<Session> & { id: string }): Session {
  return {
    organization_id: "org_1",
    harness_id: "harness_1",
    agent_id: "agent_1",
    owner_principal_id: "principal_1",
    resolved_owner_user_id: "user_1",
    title: null,
    tags: ["chat"],
    model_id: null,
    status: "idle",
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
    started_at: null,
    finished_at: null,
    ...overrides,
  } as Session;
}

describe("selectChatThreads", () => {
  it("keeps only tagged threads", () => {
    const threads = selectChatThreads(
      [
        session({ id: "sess_chat" }),
        session({ id: "sess_legacy", tags: ["global-chat", "user:1"] }),
        session({ id: "sess_run", tags: [] }),
        session({ id: "sess_eval", tags: ["eval"] }),
      ],
      "user_1",
    );

    expect(threads.map((thread) => thread.id)).toEqual(["sess_chat", "sess_legacy"]);
  });

  it("orders by last activity, most recent first", () => {
    const threads = selectChatThreads(
      [
        session({ id: "old", updated_at: "2026-08-01T00:00:00Z" }),
        session({ id: "newest", updated_at: "2026-08-09T12:00:00Z" }),
        session({ id: "middle", updated_at: "2026-08-05T00:00:00Z" }),
      ],
      "user_1",
    );

    expect(threads.map((thread) => thread.id)).toEqual(["newest", "middle", "old"]);
  });

  it("keeps pinned threads ahead of newer unpinned threads", () => {
    const threads = selectChatThreads(
      [
        session({ id: "newest", updated_at: "2026-08-09T12:00:00Z" }),
        session({ id: "pinned-old", updated_at: "2026-08-01T00:00:00Z", is_pinned: true }),
        session({ id: "pinned-new", updated_at: "2026-08-05T00:00:00Z", is_pinned: true }),
      ],
      "user_1",
    );

    expect(threads.map((thread) => thread.id)).toEqual(["pinned-new", "pinned-old", "newest"]);
  });

  it("drops threads owned by someone else", () => {
    const threads = selectChatThreads(
      [
        session({ id: "mine" }),
        session({ id: "theirs", resolved_owner_user_id: "user_2" }),
        session({ id: "unowned", resolved_owner_user_id: null }),
      ],
      "user_1",
    );

    expect(threads.map((thread) => thread.id)).toEqual(["mine", "unowned"]);
  });

  it("drops archived threads unless asked for them", () => {
    const sessions = [
      session({ id: "live" }),
      session({ id: "archived", archived_at: "2026-08-09T00:00:00Z" }),
    ];

    expect(selectChatThreads(sessions, "user_1").map((thread) => thread.id)).toEqual(["live"]);
    expect(
      selectChatThreads(sessions, "user_1", { includeArchived: true }).map((thread) => thread.id),
    ).toEqual(["live", "archived"]);
  });

  it("sorts archived threads behind live ones even when more recent", () => {
    const threads = selectChatThreads(
      [
        session({
          id: "archived-new",
          updated_at: "2026-08-09T12:00:00Z",
          archived_at: "2026-08-09T12:00:00Z",
        }),
        session({ id: "live-old", updated_at: "2026-08-01T00:00:00Z" }),
      ],
      "user_1",
      { includeArchived: true },
    );

    expect(threads.map((thread) => thread.id)).toEqual(["live-old", "archived-new"]);
  });

  it("treats every thread as mine when auth is off", () => {
    const threads = selectChatThreads(
      [session({ id: "a" }), session({ id: "b", resolved_owner_user_id: "user_2" })],
      undefined,
    );

    expect(threads).toHaveLength(2);
  });
});

describe("threadActivityAt", () => {
  it("takes the latest timestamp the session carries", () => {
    const value = threadActivityAt(
      session({
        id: "s",
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-02T00:00:00Z",
        finished_at: "2026-08-03T00:00:00Z",
      }),
    );

    expect(value).toBe(Date.parse("2026-08-03T00:00:00Z"));
  });
});

describe("threadTitle", () => {
  it("prefers the title", () => {
    expect(threadTitle({ title: "Standup", preview: "hello" })).toBe("Standup");
  });

  it("falls back to the first message preview", () => {
    expect(threadTitle({ title: null, preview: "  what broke last night?  " })).toBe(
      "what broke last night?",
    );
  });

  it("truncates a long preview", () => {
    const preview = "x".repeat(100);
    expect(threadTitle({ title: null, preview })).toBe(`${"x".repeat(60)}…`);
  });

  it("names an empty thread", () => {
    expect(threadTitle({ title: "   ", preview: null })).toBe("New chat");
  });
});
