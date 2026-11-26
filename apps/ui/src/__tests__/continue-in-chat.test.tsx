import { act, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useRef, useState, type ReactNode } from "react";
import { ContinueInChat } from "@/components/chat/continue-in-chat";
import { useChatHandoff } from "@/hooks/use-chat-handoff";
import { queueChatHandoff, takeChatHandoff } from "@/lib/chat-handoff";
import { chatRecoveryKey, clearChatRecovery } from "@/lib/chat-recovery";
import { api } from "@/lib/api/client";

const push = jest.fn();
const beside = jest.fn();
let mockBeside: typeof beside | undefined;
let mockOrg = "org_one";
jest.mock("next/navigation", () => ({ useRouter: () => ({ push }) }));
jest.mock("@/providers/auth-provider", () => ({ useAuth: () => ({ user: { id: "person" } }) }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: mockOrg } }),
}));
jest.mock("@/components/chat-mode/chat-mode-context", () => ({
  useChatMode: () => ({ openBeside: mockBeside }),
}));
jest.mock("@/lib/api/client", () => ({ api: { post: jest.fn() } }));
const post = jest.mocked(api.post);
const issue = {
  id: "issue-one",
  title: "Unattended spend jumped",
  summary: "Housekeeping spent $6.61.",
  href: "/settings/health?issue=issue-one",
  agent_id: "agent_one",
};
const session = "session_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>;
}
beforeEach(() => {
  jest.clearAllMocks();
  clearChatRecovery();
  mockBeside = undefined;
  mockOrg = "org_one";
  post.mockResolvedValue({ data: { id: session } } as never);
});

test("opens Main chat with a scoped editable issue draft and sends no message", async () => {
  render(<ContinueInChat issue={issue} />, { wrapper });
  fireEvent.click(screen.getByRole("button", { name: "Continue in chat" }));
  await waitFor(() => expect(push).toHaveBeenCalledWith("/chats"));
  expect(post).toHaveBeenCalledTimes(1);
  expect(post).toHaveBeenCalledWith("/v1/platform/sessions/platform-chat", {});
  expect(takeChatHandoff(chatRecoveryKey("someone-else", "org_one", session))).toBeUndefined();
  expect(takeChatHandoff(chatRecoveryKey("person", "org_other", session))).toBeUndefined();
  const text = takeChatHandoff(chatRecoveryKey("person", "org_one", session));
  expect(text).toContain(issue.title);
  expect(text).toContain(issue.href);
  expect(text).toContain(issue.agent_id);
  expect(takeChatHandoff(chatRecoveryKey("person", "org_one", session))).toBeUndefined();
});

test("keeps the current page when Chat can open beside it", async () => {
  mockBeside = beside;
  render(<ContinueInChat issue={issue} />, { wrapper });
  fireEvent.click(screen.getByRole("button", { name: "Continue in chat" }));
  await waitFor(() => expect(beside).toHaveBeenCalledWith(session));
  expect(push).not.toHaveBeenCalled();
});

test("preserves the issue on an opening error and can retry", async () => {
  post.mockRejectedValueOnce(new Error("unavailable"));
  render(<ContinueInChat issue={issue} />, { wrapper });
  fireEvent.click(screen.getByRole("button", { name: "Continue in chat" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open Platform Chat");
  expect(push).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Continue in chat" }));
  await waitFor(() => expect(push).toHaveBeenCalledWith("/chats"));
});

test("an organization change during opening cannot hand context to the new organization", async () => {
  let finish!: (value: unknown) => void;
  post.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve;
    }) as never,
  );
  const { rerender } = render(<ContinueInChat issue={issue} />, { wrapper });
  fireEvent.click(screen.getByRole("button", { name: "Continue in chat" }));
  mockOrg = "org_other";
  rerender(<ContinueInChat issue={issue} />);
  await act(async () => finish({ data: { id: session } }));
  expect(push).not.toHaveBeenCalled();
  expect(takeChatHandoff(chatRecoveryKey("person", "org_one", session))).toBeUndefined();
});

test("a mounted or newly opened composer appends context once and preserves existing draft text", () => {
  const key = chatRecoveryKey("person", "org_one", session);
  queueChatHandoff(key, "first issue");
  const { result } = renderHook(() => {
    const [draft, setDraft] = useState("Existing unsent question");
    const textarea = useRef<HTMLTextAreaElement>(null);
    useChatHandoff(key, setDraft, textarea);
    return draft;
  });
  expect(result.current).toBe("Existing unsent question\n\nfirst issue");
  act(() => queueChatHandoff(key, "second issue"));
  expect(result.current).toBe("Existing unsent question\n\nfirst issue\n\nsecond issue");
  expect(takeChatHandoff(key)).toBeUndefined();
});

test("logout retires queued drafts and context is bounded", () => {
  const key = chatRecoveryKey("person", "org_one", session);
  queueChatHandoff(key, "private issue");
  clearChatRecovery();
  expect(takeChatHandoff(key)).toBeUndefined();
  expect(() => queueChatHandoff(key, "x".repeat(17_000))).toThrow("too large");
});
