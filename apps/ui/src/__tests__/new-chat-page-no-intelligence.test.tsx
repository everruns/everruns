import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import NewChatPageClient from "@/app/(main)/chats/new/new-chat-page-client";

jest.mock("@/hooks/use-turn-work-report", () => ({
  useTurnWorkReport: () => null,
  useLiveWorkReport: () => null,
}));
jest.mock("@/components/chat/work-headline", () => ({
  WorkHeadline: ({ fallback }: { fallback: unknown }) => fallback,
}));
const mockCreate = jest.fn();
const mockSend = jest.fn();
const mockReplace = jest.fn();
let mockSubmitError: string | undefined;
let mockChatModel: string | undefined;
let mockAgentModel: string | undefined;
let mockModels: Array<{ id: string; enabled: boolean; healthy: boolean; capabilities: string[] }> =
  [];
let mockDraftModel: { id: string } | undefined;
jest.mock("next/navigation", () => ({ useRouter: () => ({ replace: mockReplace }) }));
jest.mock("@/hooks/use-organizations", () => ({
  useOrganization: () => ({ data: { chat_model_id: mockChatModel } }),
}));
jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({
    data: {
      id: "agent_platform",
      default_model_id: mockAgentModel,
      name: "platform-chat",
      intro_markdown: "Hello",
      starters: [{ text: "Show agents" }],
    },
  }),
}));
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({ data: mockModels }),
  useDefaultModel: () => ({ data: { id: "model-agent" } }),
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
  SessionProvider: ({
    children,
    draftModel,
  }: {
    children: React.ReactNode;
    draftModel?: { id: string };
  }) => {
    mockDraftModel = draftModel;
    return <>{children}</>;
  },
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
  mockChatModel = undefined;
  mockAgentModel = undefined;
  mockModels = [];
  mockDraftModel = undefined;
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

it("previews the independent Chat default while preserving explicit Agent selection", () => {
  mockChatModel = "model-chat";
  mockModels = ["model-agent", "model-chat"].map((id) => ({
    id,
    enabled: true,
    healthy: true,
    capabilities: [],
  }));
  const { rerender } = render(<NewChatPageClient />);
  expect(mockDraftModel?.id).toBe("model-chat");
  mockAgentModel = "model-agent";
  rerender(<NewChatPageClient />);
  expect(mockDraftModel?.id).toBe("model-agent");
  expect(mockCreate).not.toHaveBeenCalled();
});
