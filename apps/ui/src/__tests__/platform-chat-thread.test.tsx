/**
 * The Platform Chat thread is precreated and pinned so a new user lands in a
 * conversation rather than an empty Chats list — and is adopted, never
 * duplicated, when one already exists.
 */
import { render, waitFor } from "@testing-library/react";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { CHAT_THREAD_TAG } from "@/lib/chat-threads";
import type { Session } from "@/lib/api/types";

const mockCreate = jest.fn();
const mockPin = jest.fn();

jest.mock("@/hooks/use-chat-threads", () => ({
  useChatThreads: jest.fn(),
}));

jest.mock("@/hooks", () => ({
  useHarnesses: () => ({
    data: [
      { id: "harness_generic", name: "generic" },
      { id: "harness_platform", name: "platform-chat" },
    ],
    isLoading: false,
  }),
}));

jest.mock("@/hooks/use-sessions", () => ({
  useCreateSession: () => ({ mutateAsync: mockCreate }),
  usePinSession: () => ({ mutateAsync: mockPin }),
}));

// The ensure guard is keyed by org and lives for the page load, so each test
// runs against its own org — the same isolation a fresh page load gives.
let currentOrgId = "org_1";
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: currentOrgId, name: "Acme", role: "owner" } }),
}));

let orgCounter = 0;

const mockUseChatThreads = useChatThreads as jest.MockedFunction<typeof useChatThreads>;

function thread(overrides: Partial<Session>): Session {
  return {
    id: "ses_existing",
    organization_id: currentOrgId,
    harness_id: "harness_platform",
    agent_id: null,
    owner_principal_id: "user_1",
    title: "Platform Chat",
    tags: [CHAT_THREAD_TAG],
    model_id: null,
    status: "idle",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    started_at: null,
    finished_at: null,
    ...overrides,
  } as Session;
}

function Probe({ ensure }: { ensure?: boolean }) {
  const { thread: found } = usePlatformChatThread({ ensure });
  return <span data-testid="thread">{found?.id ?? "none"}</span>;
}

beforeEach(() => {
  orgCounter += 1;
  currentOrgId = `org_${orgCounter}`;
  jest.clearAllMocks();
  mockCreate.mockResolvedValue({ id: "ses_new" });
  mockPin.mockResolvedValue(undefined);
});

test("creates and pins a Platform Chat thread when the user has none", async () => {
  mockUseChatThreads.mockReturnValue({ threads: [], isLoading: false, error: null });

  render(<Probe ensure />);

  await waitFor(() => expect(mockCreate).toHaveBeenCalledTimes(1));
  expect(mockCreate).toHaveBeenCalledWith({
    request: {
      harness_name: "platform-chat",
      title: "Platform Chat",
      tags: [CHAT_THREAD_TAG],
    },
  });
  await waitFor(() => expect(mockPin).toHaveBeenCalledWith({ sessionId: "ses_new" }));
});

test("adopts an existing Platform Chat thread instead of creating a second", async () => {
  mockUseChatThreads.mockReturnValue({
    threads: [thread({})],
    isLoading: false,
    error: null,
  });

  const { getByTestId } = render(<Probe ensure />);

  expect(getByTestId("thread")).toHaveTextContent("ses_existing");
  await waitFor(() => expect(mockCreate).not.toHaveBeenCalled());
});

test("does not recreate a thread the user archived", async () => {
  mockUseChatThreads.mockReturnValue({
    threads: [thread({ archived_at: "2026-02-01T00:00:00Z" })],
    isLoading: false,
    error: null,
  });

  render(<Probe ensure />);

  await waitFor(() => expect(mockCreate).not.toHaveBeenCalled());
});

test("ignores threads bound to another harness or to an agent", async () => {
  mockUseChatThreads.mockReturnValue({
    threads: [
      thread({ id: "ses_generic", harness_id: "harness_generic" }),
      thread({ id: "ses_agent", agent_id: "agent_1" }),
    ],
    isLoading: false,
    error: null,
  });

  const { getByTestId } = render(<Probe ensure />);

  expect(getByTestId("thread")).toHaveTextContent("none");
  await waitFor(() => expect(mockCreate).toHaveBeenCalledTimes(1));
});

test("reads without creating when ensure is off", async () => {
  mockUseChatThreads.mockReturnValue({ threads: [], isLoading: false, error: null });

  render(<Probe />);

  await waitFor(() => expect(mockCreate).not.toHaveBeenCalled());
});

test("waits for the thread list before deciding to create", async () => {
  mockUseChatThreads.mockReturnValue({ threads: [], isLoading: true, error: null });

  render(<Probe ensure />);

  await waitFor(() => expect(mockCreate).not.toHaveBeenCalled());
});
