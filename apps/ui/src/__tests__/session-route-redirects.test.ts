const mockRedirect = jest.fn();

jest.mock("next/navigation", () => ({
  redirect: (href: string) => mockRedirect(href),
}));

import ApprovalsPage from "@/app/(main)/approvals/page";
import LegacySessionChatPage from "@/app/(main)/sessions/[sessionId]/chat/page";
import SessionPage from "@/app/(main)/sessions/[sessionId]/page";
import TimelinePage from "@/app/(main)/sessions/[sessionId]/timeline/page";
import TranscriptPage from "@/app/(main)/sessions/[sessionId]/transcript/page";

describe("session recording redirects", () => {
  beforeEach(() => {
    mockRedirect.mockClear();
  });

  it("lands the base session route on Trace", async () => {
    await SessionPage({ params: Promise.resolve({ sessionId: "session_123" }) });

    expect(mockRedirect).toHaveBeenCalledWith("/sessions/session_123/trace");
  });

  it("sends the retired Transcript and Timeline tabs to Trace", async () => {
    for (const page of [TranscriptPage, TimelinePage]) {
      mockRedirect.mockClear();
      await page({ params: Promise.resolve({ sessionId: "session_123" }) });
      expect(mockRedirect).toHaveBeenCalledWith("/sessions/session_123/trace");
    }
  });

  it("sends the retired org-wide approvals list to Sessions", () => {
    ApprovalsPage();

    expect(mockRedirect).toHaveBeenCalledWith("/sessions");
  });

  it("preserves legacy session chat bookmarks as Trace", async () => {
    await LegacySessionChatPage({ params: Promise.resolve({ sessionId: "session_123" }) });

    expect(mockRedirect).toHaveBeenCalledWith("/sessions/session_123/trace");
  });
});
