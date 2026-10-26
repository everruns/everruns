// EVE-855: retired top-level routes must resolve to their replacement rather
// than 404, so bookmarks and links in docs, Slack and old sessions keep working.
// Verified by test rather than by clicking.
const redirect = jest.fn();
jest.mock("next/navigation", () => ({ redirect: (path: string) => redirect(path) }));

import DashboardPage from "@/app/(main)/dashboard/page";
import ChatPage from "@/app/(main)/chat/page";
import AgentExperiencePage from "@/app/(main)/settings/agent-experience/page";

describe("retired routes", () => {
  beforeEach(() => redirect.mockClear());

  it("sends /dashboard to the landing surface", () => {
    DashboardPage();
    expect(redirect).toHaveBeenCalledWith("/chats");
  });

  it("sends the old singleton /chat to the Chats surface", () => {
    ChatPage();
    expect(redirect).toHaveBeenCalledWith("/chats");
  });

  it("sends My agent experience to Profile", async () => {
    await AgentExperiencePage({ searchParams: Promise.resolve({}) });
    expect(redirect).toHaveBeenCalledWith("/settings/user-profile");
  });

  it("sends an OAuth return on My agent experience to Connections", async () => {
    await AgentExperiencePage({
      searchParams: Promise.resolve({ connected: "github" }),
    });
    expect(redirect).toHaveBeenCalledWith("/settings/connections?connected=github");
  });

  it("sends a failed OAuth return on My agent experience to Connections", async () => {
    await AgentExperiencePage({
      searchParams: Promise.resolve({
        connect_error: "provider_refused",
        provider: "github",
      }),
    });
    expect(redirect).toHaveBeenCalledWith(
      "/settings/connections?connect_error=provider_refused&provider=github",
    );
  });
});
