import { act, renderHook } from "@testing-library/react";
import { useScrollManager } from "../use-scroll-manager";

it("scrolls the transcript without moving its enclosing workspace", () => {
  const { result } = renderHook(() =>
    useScrollManager({
      eventCount: 0,
      eventsLoaded: false,
      hasMoreEvents: false,
      loadingOlderEvents: false,
      loadOlderEvents: jest.fn(),
      sessionId: "session_1",
    }),
  );
  const workspace = document.createElement("div");
  const transcript = document.createElement("div");
  const end = document.createElement("div");
  workspace.append(transcript);
  transcript.append(end);
  Object.defineProperty(transcript, "scrollHeight", { value: 1200 });
  Object.defineProperty(transcript, "clientHeight", { value: 400 });
  transcript.scrollTo = jest.fn((options?: ScrollToOptions | number) => {
    transcript.scrollTop = Math.min(
      typeof options === "number" ? options : (options?.top ?? 0),
      800,
    );
  });
  // Native scrollIntoView may scroll every ancestor, including a split shell.
  end.scrollIntoView = jest.fn(() => {
    transcript.scrollTop = 800;
    workspace.scrollTop = 200;
  });
  result.current.scrollContainerRef.current = transcript;
  result.current.messagesEndRef.current = end;
  act(() => result.current.scrollToBottom("instant"));
  expect(transcript.scrollTop).toBe(800);
  expect(workspace.scrollTop).toBe(0);
});
