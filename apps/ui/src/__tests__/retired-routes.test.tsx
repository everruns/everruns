// EVE-855: retired top-level routes must resolve to their replacement rather
// than 404, so bookmarks and links in docs, Slack and old sessions keep working.
// Verified by test rather than by clicking.
const redirect = jest.fn();
jest.mock("next/navigation", () => ({ redirect: (path: string) => redirect(path) }));

import DashboardPage from "@/app/(main)/dashboard/page";
import ChatPage from "@/app/(main)/chat/page";
import ConnectionsPage from "@/app/(main)/settings/connections/page";

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

  it("sends Settings connections to My agent experience", async () => {
    await ConnectionsPage({ searchParams: Promise.resolve({}) });
    expect(redirect).toHaveBeenCalledWith("/settings/agent-experience");
  });

  it("keeps the OAuth return query when leaving Settings connections", async () => {
    await ConnectionsPage({
      searchParams: Promise.resolve({ connected: "github" }),
    });
    expect(redirect).toHaveBeenCalledWith("/settings/agent-experience?connected=github");
  });
});
