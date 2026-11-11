import type { RoomEvent, RoomMember, RoomMessage } from "@/lib/api/rooms";
import {
  countMembers,
  draftWakes,
  isPlaygroundRoom,
  listensByMessage,
  mentionedHandles,
  roomTotals,
  sameUserId,
  sessionsByMember,
  wakesByMessage,
} from "@/lib/rooms";

const member = (
  id: string,
  kind: RoomMember["kind"],
  handle: string,
  extra: Partial<RoomMember> = {},
): RoomMember => ({
  id,
  room_id: "room_1",
  kind,
  handle,
  display_name: handle,
  role: "member",
  wake: "mentions",
  session_scope: "thread",
  left: false,
  joined_at: "2026-11-10T10:00:00Z",
  ...extra,
});

const message = (id: string, extra: Partial<RoomMessage> = {}): RoomMessage => ({
  id,
  room_id: "room_1",
  text: "",
  mentions: [],
  reactions: [],
  reply_count: 0,
  pinned: false,
  deleted: false,
  hops: 0,
  created_at: "2026-11-10T10:00:00Z",
  ...extra,
});

const event = (seq: number, type: string, extra: Partial<RoomEvent> = {}): RoomEvent => ({
  seq,
  room_id: "room_1",
  type,
  data: {},
  created_at: "2026-11-10T10:00:00Z",
  ...extra,
});

const members = [
  member("rmem_ana", "person", "ana", { user_id: "0193-ab" }),
  member("rmem_plan", "agent", "planning"),
  member("rmem_ops", "agent", "ops", { wake: "everything" }),
  member("rmem_alice", "guest", "alice-test"),
  member("rmem_old", "agent", "old", { left: true }),
];

describe("rooms in the console", () => {
  it("tells Playground rooms by their metadata", () => {
    expect(isPlaygroundRoom({ metadata: { playground: true } })).toBe(true);
    expect(isPlaygroundRoom({ metadata: { playground: "yes" } })).toBe(false);
    expect(isPlaygroundRoom({ metadata: null })).toBe(false);
  });

  it("counts the members who are still in the room, by kind", () => {
    expect(countMembers(members)).toEqual({ agents: 2, people: 1, outsideAgents: 0, guests: 1 });
  });

  it("says why each agent woke, and which a limit stopped", () => {
    const top = message("rmsg_1", { mentions: [{ id: "rmem_plan", handle: "planning" }] });
    const reply = message("rmsg_2", { thread_id: "rmsg_1" });
    const wakes = wakesByMessage(
      [
        event(1, "message.posted", { message_id: "rmsg_1", member_id: "rmem_ana" }),
        event(2, "agent.woken", {
          message_id: "rmsg_1",
          member_id: "rmem_plan",
          data: { session_id: "session_a" },
        }),
        event(3, "agent.woken", {
          message_id: "rmsg_1",
          member_id: "rmem_ops",
          data: { session_id: "session_b" },
        }),
        event(4, "agent.woken", {
          message_id: "rmsg_2",
          member_id: "rmem_plan",
          data: { session_id: "session_a" },
        }),
        event(5, "agent.limited", {
          message_id: "rmsg_2",
          member_id: "rmem_ops",
          data: { limit: "hops" },
        }),
      ],
      [top, reply],
      members,
    );
    expect(wakes.get("rmsg_1")?.map((wake) => [wake.handle, wake.reason, wake.sessionId])).toEqual([
      ["planning", "mentioned", "session_a"],
      ["ops", "everything", "session_b"],
    ]);
    expect(wakes.get("rmsg_2")?.map((wake) => [wake.handle, wake.reason, wake.limit])).toEqual([
      ["planning", "thread", undefined],
      ["ops", "limited", "hops"],
    ]);
  });

  it("shows the reason the server stored, with a relevant wake's confidence", () => {
    const listener = member("rmem_sre", "agent", "srebot", { wake: "relevant" });
    const all = [...members, listener];
    const top = message("rmsg_1", { mentions: [{ id: "rmem_plan", handle: "planning" }] });
    const events = [
      event(1, "agent.considered", {
        message_id: "rmsg_1",
        member_id: "rmem_sre",
        data: { decision: "woke", probability: 0.94, judge: "deployment" },
      }),
      event(2, "agent.woken", {
        message_id: "rmsg_1",
        member_id: "rmem_sre",
        data: { session_id: "session_s", reason: "relevant" },
      }),
      event(3, "agent.considered", {
        message_id: "rmsg_1",
        member_id: "rmem_ops",
        data: { decision: "quiet", probability: 0.31 },
      }),
      event(4, "agent.considered", {
        message_id: "rmsg_1",
        member_id: "rmem_plan",
        data: { decision: "quiet", error: "unavailable" },
      }),
    ];
    const wakes = wakesByMessage(events, [top], all).get("rmsg_1");
    expect(wakes?.map((wake) => [wake.handle, wake.reason, wake.probability])).toEqual([
      ["srebot", "relevant", 0.94],
    ]);
    const listens = listensByMessage(events, all).get("rmsg_1");
    expect(
      listens?.map((listen) => [listen.handle, listen.probability, listen.unavailable]),
    ).toEqual([
      ["ops", 0.31, false],
      ["planning", undefined, true],
    ]);
  });

  it("lists each agent's sessions once, in the order they started", () => {
    const sessions = sessionsByMember([
      event(1, "agent.woken", { member_id: "rmem_plan", data: { session_id: "session_a" } }),
      event(2, "agent.woken", { member_id: "rmem_plan", data: { session_id: "session_b" } }),
      event(3, "agent.woken", { member_id: "rmem_plan", data: { session_id: "session_a" } }),
    ]);
    expect(sessions.get("rmem_plan")).toEqual(["session_a", "session_b"]);
  });

  it("totals messages, agent posts, wakes and limits", () => {
    expect(
      roomTotals(
        [
          event(1, "message.posted", { member_id: "rmem_ana" }),
          event(2, "agent.woken", { member_id: "rmem_plan" }),
          event(3, "message.posted", { member_id: "rmem_plan" }),
          event(4, "agent.limited", { member_id: "rmem_ops" }),
        ],
        members,
      ),
    ).toEqual({ messages: 2, agentMessages: 1, wakes: 1, limited: 1 });
  });

  it("finds mentions the way the server writes handles, not inside emails", () => {
    expect(mentionedHandles("@Planning and @ops-2, mail ana@example.com")).toEqual([
      "planning",
      "ops-2",
    ]);
  });

  it("says before posting who a draft wakes", () => {
    const draft = draftWakes("@planning @old @alice-test what now?", members, false);
    expect(draft.mentioned.map((agent) => agent.handle)).toEqual(["planning"]);
    expect(draft.everything.map((agent) => agent.handle)).toEqual(["ops"]);
    expect(draft.thread).toBe(false);
    const listener = member("rmem_sre", "agent", "srebot", { wake: "relevant" });
    expect(
      draftWakes("checkout is down", [...members, listener], false).listening.map((a) => a.handle),
    ).toEqual(["srebot"]);
    expect(draftWakes("@srebot look", [...members, listener], false).listening).toEqual([]);
  });

  it("compares prefixed and raw user ids", () => {
    expect(sameUserId("user_0193ab", "0193-AB")).toBe(true);
    expect(sameUserId("user_0193ab", "0193ac")).toBe(false);
    expect(sameUserId(undefined, "0193ab")).toBe(false);
  });
});
