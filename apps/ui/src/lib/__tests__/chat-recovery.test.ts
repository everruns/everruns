import {
  chatRecoveryKey,
  clearChatRecovery,
  readChatDraft,
  readPendingSends,
  writeChatRecovery,
  MAX_RECOVERY_BYTES,
} from "../chat-recovery";
import type { PendingSend } from "../chat-turn-state";
const key = chatRecoveryKey("alice", "org", "session");
const item: PendingSend = {
  clientId: "019a0000-0000-7000-8000-000000000001",
  text: "hi",
  images: [],
  files: [],
  sentAtMs: 1,
  phase: "sending",
};
beforeEach(() => sessionStorage.clear());
it("saves draft and sends without overwriting either", () => {
  writeChatRecovery(key, { draft: "draft" });
  writeChatRecovery(key, { pending: [item] });
  expect(readChatDraft(key)).toBe("draft");
  expect(readPendingSends(key)[0].clientId).toBe(item.clientId);
  writeChatRecovery(key, { pending: [] });
  expect(readChatDraft(key)).toBe("draft");
});
it.each(["{", "null", '{"pending":{}}', '{"pending":[{"clientId":"bad"}]}'])(
  "rejects corrupt storage without executing or erasing it: %s",
  (value) => {
    sessionStorage.setItem(key, value);
    expect(() => readPendingSends(key)).toThrow();
    expect(sessionStorage.getItem(key)).toBe(value);
  },
);
it("bounds encoded bytes and pending count without replacing the last saved draft", () => {
  writeChatRecovery(key, { draft: "saved" });
  expect(() => writeChatRecovery(key, { draft: "😀".repeat(MAX_RECOVERY_BYTES / 3) })).toThrow();
  expect(() => writeChatRecovery(key, { pending: Array(51).fill(item) })).toThrow();
  expect(readChatDraft(key)).toBe("saved");
});
it("logout clears every recovery scope and preserves unrelated preferences", () => {
  writeChatRecovery(key, { draft: "alice" });
  writeChatRecovery(chatRecoveryKey("bob", "other", "other"), { draft: "bob" });
  sessionStorage.setItem("theme", "dark");
  clearChatRecovery();
  expect(sessionStorage.length).toBe(1);
  expect(sessionStorage.getItem("theme")).toBe("dark");
});

it("marks expired restored requests review-only without extending their original age", () => {
  writeChatRecovery(key, { pending: [{ ...item, createdAtMs: 1, sentAtMs: Date.now() }] });
  expect(readPendingSends(key)[0]).toMatchObject({
    createdAtMs: 1,
    retryExpired: true,
    retryWhenOnline: false,
  });
});
