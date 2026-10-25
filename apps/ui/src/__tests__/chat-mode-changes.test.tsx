import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { EntityChange } from "@/lib/api/types";
import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import { useAgentChatChanges } from "@/components/chat-mode/use-agent-chat-changes";
import {
  diffEntityRevisions,
  listEntityHistory,
  restoreEntityRevision,
} from "@/lib/api/change-history";

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" } }),
}));
jest.mock("@/lib/api/change-history", () => ({
  listEntityHistory: jest.fn(),
  diffEntityRevisions: jest.fn(),
  restoreEntityRevision: jest.fn(),
}));

const entry = (revision: number, session?: string): EntityChange => ({
  id: `change_${revision}`,
  revision,
  entity_kind: "agent",
  entity_ref: "agent_1",
  command: "update_agent",
  action: "updated",
  actor_kind: session ? "agent_session" : "user",
  via_session_id: session,
  surface: session ? "platform" : "api",
  changed_fields: [],
  created_at: "2026-10-18T10:00:00Z",
});
const promptDiff = { field: "system_prompt", from: "Original", to: "Kid-safe" };
const tagsDiff = { field: "tags", from: [], to: ["safe"] };
let entries: EntityChange[];

function setup() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const recordAction = jest.fn();
  const mode: ChatModeState = {
    open: true,
    setOpen: jest.fn(),
    threadId: "session_chat",
    pageContainer: { current: null },
    page: {
      route: "/agents/agent_1",
      label: "Support",
      entity_id: "agent_1",
      entity_type: "agent",
    },
    registerPage: () => () => {},
    actions: [],
    recordAction,
  };
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>
      <ChatModeContext.Provider value={mode}>{children}</ChatModeContext.Provider>
    </QueryClientProvider>
  );
  const hook = renderHook(() => useAgentChatChanges("agent_1"), { wrapper });
  return { ...hook, queryClient, recordAction };
}

beforeEach(() => {
  jest.clearAllMocks();
  entries = [entry(1)];
  jest.mocked(listEntityHistory).mockImplementation(async () => ({ entries }));
  jest.mocked(diffEntityRevisions).mockResolvedValue([promptDiff, tagsDiff]);
  jest.mocked(restoreEntityRevision).mockImplementation(async () => {
    entries = [entry(3), ...entries];
    return { restored_revision: 1, entity: {}, warnings: [] };
  });
});

async function loadBaseline(client: QueryClient) {
  await waitFor(() =>
    expect(client.getQueryState(["chat-mode-history", "org_1", "agent_1"])?.status).toBe("success"),
  );
}
async function refresh(client: QueryClient) {
  await act(async () => {
    await client.invalidateQueries({ queryKey: ["chat-mode-history"] });
  });
}

it("ignores historical and other-session changes, and follows saves from this Chat", async () => {
  const { result, queryClient } = setup();
  await loadBaseline(queryClient);
  expect(result.current.fields).toEqual([]);
  entries = [entry(2, "session_other"), ...entries];
  await refresh(queryClient);
  await waitFor(() =>
    expect(
      queryClient.getQueryData<{ entries: EntityChange[] }>([
        "chat-mode-history",
        "org_1",
        "agent_1",
      ])?.entries[0].revision,
    ).toBe(2),
  );
  expect(result.current.fields).toEqual([]);
  entries = [entry(3, "session_chat"), ...entries];
  await refresh(queryClient);
  await waitFor(() => expect(result.current.fields).toEqual([promptDiff, tagsDiff]));
  expect(diffEntityRevisions).toHaveBeenLastCalledWith("agent_1", 2, 3);
});

it("undoes one field against the saved revision and keeps the other marker", async () => {
  const { result, queryClient, recordAction } = setup();
  await loadBaseline(queryClient);
  entries = [entry(2, "session_chat"), ...entries];
  await refresh(queryClient);
  await waitFor(() => expect(result.current.fields).toHaveLength(2));
  await act(async () => {
    await result.current.undo("system_prompt");
  });
  expect(restoreEntityRevision).toHaveBeenCalledWith("agent_1", 1, expect.any(String), {
    expected_revision: 2,
    fields: ["system_prompt"],
  });
  await waitFor(() => expect(result.current.fields).toEqual([tagsDiff]));
  expect(recordAction).toHaveBeenCalledTimes(1);
});

it("keeps markers and reports a failed undo without recording success", async () => {
  jest
    .mocked(restoreEntityRevision)
    .mockRejectedValue(new Error("Agent changed since Chat saved it"));
  const { result, queryClient, recordAction } = setup();
  await loadBaseline(queryClient);
  entries = [entry(2, "session_chat"), ...entries];
  await refresh(queryClient);
  await waitFor(() => expect(result.current.fields).toHaveLength(2));
  await act(async () => {
    await result.current.undo();
  });
  expect(result.current.error).toContain("Agent changed");
  expect(result.current.fields).toHaveLength(2);
  expect(recordAction).not.toHaveBeenCalled();
});

it("dismisses markers without making a write and starts a fresh baseline", async () => {
  const { result, queryClient } = setup();
  await loadBaseline(queryClient);
  entries = [entry(2, "session_chat"), ...entries];
  await refresh(queryClient);
  await waitFor(() => expect(result.current.fields).toHaveLength(2));
  act(() => result.current.dismiss());
  expect(result.current.fields).toEqual([]);
  expect(restoreEntityRevision).not.toHaveBeenCalled();
  entries = [entry(3, "session_chat"), ...entries];
  await refresh(queryClient);
  await waitFor(() => expect(diffEntityRevisions).toHaveBeenLastCalledWith("agent_1", 2, 3));
});

it("includes all Chat saves between polls in Undo all", async () => {
  const { result, queryClient } = setup();
  await loadBaseline(queryClient);
  entries = [entry(3, "session_chat"), entry(2, "session_chat"), ...entries];
  await refresh(queryClient);
  await waitFor(() => expect(result.current.fields).toHaveLength(2));
  expect(diffEntityRevisions).toHaveBeenCalledWith("agent_1", 1, 3);
});
