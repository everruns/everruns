import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import NewChatPageClient from "@/app/(main)/chats/new/new-chat-page-client";
const mockCreate = jest.fn();
const mockSend = jest.fn();
const mockReplace = jest.fn();
let mockSubmitError: string | undefined;
jest.mock("next/navigation", () => ({ useRouter: () => ({ replace: mockReplace }) }));
jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({
    data: {
      id: "agent_platform",
      name: "platform-chat",
      intro_markdown: "Hello",
      starters: [{ text: "Show agents" }],
    },
  }),
}));
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({ data: [] }),
  useDefaultModel: () => ({ data: null }),
}));
jest.mock("@/hooks/use-sessions", () => ({
  useCreateSession: () => ({ mutateAsync: mockCreate }),
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_a" } }),
}));
jest.mock("@/lib/api/messages", () => ({
  sendUserMessageWithImages: (...args: unknown[]) => mockSend(...args),
}));
jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
jest.mock("@/components/chat/chat-panel", () => ({
  ChatPanel: ({
    onDraftSubmit,
    platformStarters,
  }: {
    onDraftSubmit: (t: string, i: unknown[], f: unknown[]) => Promise<void>;
    platformStarters: { text: string }[];
  }) => (
    <button
      onClick={() =>
        void onDraftSubmit("hello", [], []).catch((e: Error) => {
          mockSubmitError = e.message;
        })
      }
    >
      {platformStarters[0].text}
    </button>
  ),
}));
beforeEach(() => {
  jest.clearAllMocks();
  mockSubmitError = undefined;
  mockCreate.mockResolvedValue({ id: "ses_side" });
  mockSend.mockResolvedValue({});
});
it("opens an empty draft with Agent starters and creates only on first send", async () => {
  render(<NewChatPageClient />);
  expect(mockCreate).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Show agents" }));
  await waitFor(() => expect(mockReplace).toHaveBeenCalledWith("/chats/ses_side"));
  expect(mockCreate).toHaveBeenCalledWith({
    request: { source: "chat", agent_name: "platform-chat", tags: ["chat"] },
  });
  expect(mockSend).toHaveBeenCalledWith("ses_side", "hello", [], undefined, undefined, []);
});
it("reuses a created session after the first message fails", async () => {
  mockSend.mockRejectedValueOnce(new Error("offline"));
  render(<NewChatPageClient />);
  fireEvent.click(screen.getByRole("button", { name: "Show agents" }));
  await waitFor(() => expect(mockSubmitError).toBe("offline"));
  expect(mockReplace).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Show agents" }));
  await waitFor(() => expect(mockReplace).toHaveBeenCalled());
  expect(mockCreate).toHaveBeenCalledTimes(1);
});
