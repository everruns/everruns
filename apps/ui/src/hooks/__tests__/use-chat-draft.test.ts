import { act, renderHook } from "@testing-library/react";
import { useChatDraft } from "../use-chat-draft";
import { chatRecoveryKey, writeChatRecovery } from "@/lib/chat-recovery";

beforeEach(() => sessionStorage.clear());
it("restores a draft after reload without clobbering pending sends", () => {
  const key = chatRecoveryKey("alice", "org", "session");
  const first = renderHook(() => useChatDraft(key));
  act(() => first.result.current[1]("unsent text"));
  first.unmount();
  const restored = renderHook(() => useChatDraft(key));
  expect(restored.result.current[0]).toBe("unsent text");
  writeChatRecovery(key, { pending: [] });
  restored.unmount();
  expect(renderHook(() => useChatDraft(key)).result.current[0]).toBe("unsent text");
});
it("keeps account and session drafts separate, including the transition render", () => {
  const seen: string[] = [];
  const { result, rerender } = renderHook(
    ({ key }) => {
      const draft = useChatDraft(key);
      seen.push(draft[0]);
      return draft;
    },
    { initialProps: { key: chatRecoveryKey("alice", "org", "session") } },
  );
  act(() => result.current[1]("private"));
  seen.length = 0;
  rerender({ key: chatRecoveryKey("bob", "org", "session") });
  expect(seen.every((text) => text === "")).toBe(true);
});
it("keeps text editable and reports disabled storage", () => {
  const spy = jest.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new DOMException("quota", "QuotaExceededError");
  });
  const { result } = renderHook(() => useChatDraft("test"));
  act(() => result.current[1]("still here"));
  expect(result.current[0]).toBe("still here");
  expect(result.current[2]).toBe(true);
  spy.mockRestore();
});

it("ignores a late draft callback from the previous conversation", () => {
  const first = chatRecoveryKey("alice", "org", "first"),
    second = chatRecoveryKey("alice", "org", "second");
  const { result, rerender } = renderHook(({ key }) => useChatDraft(key), {
    initialProps: { key: first },
  });
  const previousSetter = result.current[1];
  rerender({ key: second });
  act(() => result.current[1]("new draft"));
  act(() => previousSetter("late old draft"));
  expect(result.current[0]).toBe("new draft");
  expect(sessionStorage.getItem(first)).toBeNull();
});
