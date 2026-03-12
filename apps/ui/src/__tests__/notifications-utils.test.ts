import {
  getActiveNotificationTarget,
  getNotificationTargetKey,
  shouldSuppressNotification,
} from "@/lib/notifications";
import type { Notification } from "@/lib/api/types";

function makeNotification(overrides: Partial<Notification> = {}): Notification {
  return {
    id: "notification_01933b5a00007000800000000000001",
    kind: "turn.long_running_completed",
    title: "Long-running turn completed",
    body: "Session finished after 1m 01s.",
    target_type: "session",
    target_id: "session_01933b5a00007000800000000000001",
    href: "/sessions/session_01933b5a00007000800000000000001/chat",
    payload: {},
    occurrence_count: 1,
    viewed_at: null,
    created_at: "2026-03-11T12:00:00.000Z",
    updated_at: "2026-03-11T12:00:00.000Z",
    ...overrides,
  };
}

describe("notification helpers", () => {
  it("extracts active session chat target from pathname", () => {
    expect(
      getActiveNotificationTarget("/sessions/session_01933b5a00007000800000000000001/chat"),
    ).toBe("session:session_01933b5a00007000800000000000001");
    expect(getActiveNotificationTarget("/sessions")).toBeNull();
  });

  it("builds notification target keys", () => {
    expect(getNotificationTargetKey(makeNotification())).toBe(
      "session:session_01933b5a00007000800000000000001",
    );
    expect(getNotificationTargetKey(makeNotification({ target_type: null }))).toBeNull();
  });

  it("suppresses only matching unread notifications when the chat is visible and focused", () => {
    const notification = makeNotification();
    expect(
      shouldSuppressNotification(
        notification,
        "session:session_01933b5a00007000800000000000001",
        true,
        true,
      ),
    ).toBe(true);
    expect(
      shouldSuppressNotification(
        notification,
        "session:session_01933b5a00007000800000000000002",
        true,
        true,
      ),
    ).toBe(false);
    expect(
      shouldSuppressNotification(
        notification,
        "session:session_01933b5a00007000800000000000001",
        false,
        true,
      ),
    ).toBe(false);
    expect(
      shouldSuppressNotification(
        notification,
        "session:session_01933b5a00007000800000000000001",
        true,
        false,
      ),
    ).toBe(false);
    expect(
      shouldSuppressNotification(
        makeNotification({ viewed_at: "2026-03-11T12:01:00.000Z" }),
        "session:session_01933b5a00007000800000000000001",
        true,
        true,
      ),
    ).toBe(false);
  });
});
