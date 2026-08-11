const mockRedirect = jest.fn();

jest.mock("next/navigation", () => ({
  redirect: (href: string) => mockRedirect(href),
}));

import LegacySessionChatPage from "@/app/(main)/sessions/[sessionId]/chat/page";
import SessionPage from "@/app/(main)/sessions/[sessionId]/page";

describe("session recording redirects", () => {
  beforeEach(() => {
    mockRedirect.mockClear();
  });

  it("lands the base session route on Transcript", async () => {
    await SessionPage({ params: Promise.resolve({ sessionId: "session_123" }) });

    expect(mockRedirect).toHaveBeenCalledWith("/sessions/session_123/transcript");
  });

  it("preserves legacy session chat bookmarks as Transcript", async () => {
    await LegacySessionChatPage({ params: Promise.resolve({ sessionId: "session_123" }) });

    expect(mockRedirect).toHaveBeenCalledWith("/sessions/session_123/transcript");
  });
});
