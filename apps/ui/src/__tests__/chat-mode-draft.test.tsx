import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { useAgentDraft } from "@/components/agents/use-agent-draft";
import { useAgentChatDraft } from "@/components/chat-mode/use-agent-chat-draft";
import { usePageDraftTools } from "@/components/chat-mode/use-page-draft-tools";
import { ChatModeContext, type ChatModeState } from "@/components/chat-mode/chat-mode-context";
import {
  PAGE_READ_TOOL,
  PAGE_PATCH_TOOL,
  MAX_PAGE_DRAFT_SIZE,
  type PageDraftAdapter,
} from "@/components/chat-mode/page-draft";
import { submitToolResults } from "@/lib/api/sessions";
import type { Agent, Event } from "@/lib/api/types";

jest.mock("@/lib/api/sessions", () => ({ submitToolResults: jest.fn() }));
// The runtime and tool contract carry JSON-safe data; jsdom lacks this browser API.
global.structuredClone = (value) => JSON.parse(JSON.stringify(value));

const agent = {
  id: "agent_1",
  name: "support",
  system_prompt: "Saved prompt",
  harness_id: "harness_1",
  capabilities: [],
  initial_files: [],
  tags: [],
  updated_at: "2026-10-19T00:00:00Z",
} as unknown as Agent;

function setup() {
  const beginEdit = jest.fn();
  return {
    beginEdit,
    ...renderHook(
      ({ writable, blocked, updatedAt }) => {
        const draft = useAgentDraft(agent);
        const chat = useAgentChatDraft({
          agentId: agent.id,
          updatedAt,
          draft,
          writable,
          blocked,
          beginEdit,
        });
        return { draft, ...chat };
      },
      { initialProps: { writable: true, blocked: false, updatedAt: agent.updated_at } },
    ),
  };
}

it("reads unsaved form values and applies an atomic draft patch without losing other edits", () => {
  const { result, beginEdit } = setup();
  act(() => result.current.draft.setField("description", "My unsaved description"));
  const adapter = result.current.adapter;
  const before = adapter.read(adapter.describe().binding);
  expect(before.dirty).toBe(true);
  expect(before.values.description).toBe("My unsaved description");
  expect(before.patch_schema).toMatchObject({
    type: "object",
    additionalProperties: false,
    properties: { system_prompt: { type: "string" }, name: { type: "string" } },
  });
  expect(
    (before.patch_schema?.properties as Record<string, unknown>).sandbox_policy,
  ).toBeUndefined();
  let revision = "";
  act(() => {
    revision = adapter.patch(before.binding, before.revision, {
      system_prompt: "Improved prompt",
      tags: "support, tested",
      capabilities: [{ ref: "current_time" }],
    }).revision;
  });
  expect(result.current.draft.fields.description).toBe("My unsaved description");
  expect(result.current.draft.fields.system_prompt).toBe("Improved prompt");
  expect(result.current.draft.capabilities).toEqual([{ ref: "current_time", config: {} }]);
  expect(result.current.changes.fields).toHaveLength(3);
  expect(beginEdit).toHaveBeenCalledTimes(1);
  expect(adapter.describe().revision).toBe(revision);
  expect(result.current.draft.buildRequest()).toMatchObject({
    ok: true,
    request: { system_prompt: "Improved prompt", tags: ["support", "tested"] },
  });
});

it("rejects stale, wrong-page, read-only and conflicting patches without changing any field", () => {
  const { result, rerender } = setup();
  const adapter = result.current.adapter;
  const before = adapter.read(adapter.describe().binding);
  act(() => result.current.draft.setField("description", "Typed while Chat was thinking"));
  expect(() =>
    adapter.patch(before.binding, before.revision, { system_prompt: "Lost edit" }),
  ).toThrow("draft changed");
  expect(() => adapter.read("another-page")).toThrow("page has changed");
  const latest = adapter.read(before.binding);
  rerender({ writable: false, blocked: false, updatedAt: agent.updated_at });
  expect(() =>
    adapter.patch(latest.binding, latest.revision, { system_prompt: "No permission" }),
  ).toThrow("read-only");
  rerender({ writable: true, blocked: true, updatedAt: agent.updated_at });
  expect(() =>
    adapter.patch(latest.binding, latest.revision, { system_prompt: "Conflict" }),
  ).toThrow("conflict");
  expect(result.current.draft.fields.system_prompt).toBe("Saved prompt");
});

it("validates every patch field before applying and bounds the result", () => {
  const { result } = setup();
  const adapter = result.current.adapter;
  const before = adapter.read(adapter.describe().binding);
  expect(() =>
    adapter.patch(before.binding, before.revision, { system_prompt: "New", credentials: [] }),
  ).toThrow();
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      initial_files: [{ path: "/a", content: "x", encoding: "bad" }],
    }),
  ).toThrow();
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      system_prompt: "a".repeat(MAX_PAGE_DRAFT_SIZE),
    }),
  ).toThrow("size limit");
  expect(result.current.draft.fields.system_prompt).toBe("Saved prompt");
});

it("undoes Chat edits to the preceding manual draft and guards later manual changes", () => {
  const { result } = setup();
  act(() => result.current.draft.setField("system_prompt", "My draft prompt"));
  const adapter = result.current.adapter;
  const before = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(before.binding, before.revision, { system_prompt: "Chat prompt", tags: "chat" });
  });
  act(() => result.current.changes.undo("system_prompt"));
  expect(result.current.draft.fields.system_prompt).toBe("My draft prompt");
  expect(result.current.draft.fields.tags).toBe("chat");
  act(() => result.current.draft.setField("tags", "manual"));
  act(() => result.current.changes.undo());
  expect(result.current.changes.error).toContain("overwrite");
  expect(result.current.draft.fields.tags).toBe("manual");
});

it("keeps a manual edit made between two Chat patches as the new Undo baseline", () => {
  const { result } = setup();
  const adapter = result.current.adapter;
  const first = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(first.binding, first.revision, { system_prompt: "Chat one" });
  });
  act(() => result.current.draft.setField("system_prompt", "Manual refinement"));
  const second = adapter.read(first.binding);
  act(() => {
    adapter.patch(second.binding, second.revision, { system_prompt: "Chat two" });
  });
  act(() => result.current.changes.undo());
  expect(result.current.draft.fields.system_prompt).toBe("Manual refinement");
});

function event(id: string, type: string, data: unknown): Event {
  return {
    id,
    type,
    data,
    context: { input_message_id: "msg_1", turn_id: "turn_1" },
    session_id: "session_chat",
    ts: "2026-10-19T00:00:00Z",
  } as Event;
}
function toolsSetup(
  calls = [
    {
      id: "call_1",
      name: PAGE_PATCH_TOOL,
      arguments: { binding: "bound", revision: "v1", changes: { system_prompt: "Chat" } },
    },
  ],
  initialEnabled = true,
) {
  const adapter: PageDraftAdapter = {
    describe: () => ({ binding: "bound", revision: "v1", dirty: false, writable: true }),
    read: jest.fn().mockReturnValue({ binding: "bound", revision: "v1" }),
    patch: jest
      .fn()
      .mockReturnValue({ binding: "bound", revision: "v2", values: { system_prompt: "Chat" } }),
  };
  const mode: ChatModeState = {
    open: true,
    setOpen: jest.fn(),
    page: { route: "/agents/agent_1", label: "Agent" },
    pageAdapter: adapter,
    pageContainer: { current: null },
    registerPage: () => () => {},
    actions: [],
    recordAction: jest.fn(),
  };
  const events = [
    event("input_1", "input.message", {
      message: { id: "msg_1", metadata: { page_context: { draft: adapter.describe() } } },
    }),
    event("request_1", "tool.call_requested", { tool_calls: calls }),
  ];
  const wrapper = ({ children }: { children: ReactNode }) => (
    <ChatModeContext.Provider value={mode}>{children}</ChatModeContext.Provider>
  );
  const initialProps: { inputEvents: Event[]; enabled?: boolean } = {
    inputEvents: events,
    enabled: initialEnabled,
  };
  return {
    adapter,
    mode,
    events,
    ...renderHook(
      ({ inputEvents, enabled = true }: { inputEvents: Event[]; enabled?: boolean }) =>
        usePageDraftTools(enabled, "session_chat", inputEvents, "waiting_for_tool_results"),
      { wrapper, initialProps },
    ),
  };
}

beforeEach(() => {
  jest.clearAllMocks();
  jest.mocked(submitToolResults).mockResolvedValue({ status: "active", tool_results_count: 1 });
});

it("retries result delivery without applying the same patch twice", async () => {
  jest.mocked(submitToolResults).mockRejectedValueOnce(new Error("offline"));
  const { result, adapter } = toolsSetup();
  await waitFor(() => expect(result.current.error).toContain("Retry"));
  act(() => result.current.retry());
  await waitFor(() => expect(result.current.error).toBeNull());
  expect(adapter.patch).toHaveBeenCalledTimes(1);
  expect(submitToolResults).toHaveBeenCalledTimes(2);
  expect(jest.mocked(submitToolResults).mock.calls[1]).toEqual(
    jest.mocked(submitToolResults).mock.calls[0],
  );
  expect(jest.mocked(submitToolResults).mock.calls[0][1][0].result).toMatchObject({ saved: false });
});

it("does not execute another browser's tools, abandoned requests, or completed requests", async () => {
  const { adapter, events, rerender } = toolsSetup([
    {
      id: "call_1",
      name: PAGE_READ_TOOL,
      arguments: { binding: "bound", revision: "", changes: { system_prompt: "" } },
    },
  ]);
  await waitFor(() => expect(submitToolResults).toHaveBeenCalledTimes(1));
  const foreign = event("input_2", "input.message", {
    message: { id: "msg_1", metadata: { page_context: { draft: { binding: "another-browser" } } } },
  });
  rerender({ inputEvents: [foreign, { ...events[1], id: "foreign_request" }] });
  rerender({
    inputEvents: [...events, event("new-input", "input.message", { message: { id: "msg_2" } })],
  });
  rerender({
    inputEvents: [...events, event("done", "tool.completed", { tool_call_id: "call_1" })],
  });
  expect(adapter.patch).not.toHaveBeenCalled();
  expect(adapter.read).toHaveBeenCalledTimes(1);
});

it("rejects mixed client batches without patching the page or answering a question", async () => {
  const { adapter } = toolsSetup([
    {
      id: "call_1",
      name: PAGE_PATCH_TOOL,
      arguments: { binding: "bound", revision: "v1", changes: { system_prompt: "Chat" } },
    },
    {
      id: "ask",
      name: "ask_user",
      arguments: { binding: "", revision: "", changes: { system_prompt: "" } },
    },
  ]);
  await waitFor(() => expect(submitToolResults).toHaveBeenCalledTimes(1));
  expect(adapter.patch).not.toHaveBeenCalled();
  expect(jest.mocked(submitToolResults).mock.calls[0][1]).toEqual([
    expect.objectContaining({
      tool_call_id: "call_1",
      error: expect.stringContaining("separately"),
    }),
    expect.objectContaining({ tool_call_id: "ask", error: expect.stringContaining("separately") }),
  ]);
});

it("returns an unavailable-page error when the originating browser navigates away", async () => {
  const { adapter, mode, events, rerender } = toolsSetup([]);
  mode.pageAdapter = undefined;
  const request = event("new_request", "tool.call_requested", {
    tool_calls: [
      {
        id: "late",
        name: PAGE_PATCH_TOOL,
        arguments: { binding: "bound", revision: "v1", changes: { system_prompt: "Wrong page" } },
      },
    ],
  });
  rerender({ inputEvents: [events[0], request] });
  await waitFor(() => expect(submitToolResults).toHaveBeenCalledTimes(1));
  expect(adapter.patch).not.toHaveBeenCalled();
  expect(jest.mocked(submitToolResults).mock.calls[0][1][0]).toMatchObject({
    error: expect.stringContaining("no longer available"),
  });
});

it("executes a new request that arrives while the preceding result is being delivered", async () => {
  let finishDelivery!: (value: Awaited<ReturnType<typeof submitToolResults>>) => void;
  jest.mocked(submitToolResults).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishDelivery = resolve;
      }),
  );
  const { adapter, events, rerender } = toolsSetup();
  expect(adapter.patch).toHaveBeenCalledTimes(1);
  rerender({
    inputEvents: [
      ...events,
      event("done", "tool.completed", { tool_call_id: "call_1" }),
      event("request_2", "tool.call_requested", {
        tool_calls: [{ id: "call_2", name: PAGE_READ_TOOL, arguments: { binding: "bound" } }],
      }),
    ],
  });
  expect(adapter.read).not.toHaveBeenCalled();
  await act(async () => finishDelivery({ status: "active", tool_results_count: 1 }));
  await waitFor(() => expect(adapter.read).toHaveBeenCalledTimes(1));
  expect(adapter.patch).toHaveBeenCalledTimes(1);
  expect(submitToolResults).toHaveBeenCalledTimes(2);
});

it("answers an owned page request after switching to full Chat without patching another page", async () => {
  const { adapter, mode, events, rerender } = toolsSetup([]);
  mode.pageAdapter = undefined;
  mode.open = false;
  rerender({
    enabled: false,
    inputEvents: [
      events[0],
      event("late_request", "tool.call_requested", {
        tool_calls: [
          {
            id: "late",
            name: PAGE_PATCH_TOOL,
            arguments: {
              binding: "bound",
              revision: "v1",
              changes: { system_prompt: "Wrong page" },
            },
          },
        ],
      }),
    ],
  });
  await waitFor(() => expect(submitToolResults).toHaveBeenCalledTimes(1));
  expect(adapter.patch).not.toHaveBeenCalled();
  expect(jest.mocked(submitToolResults).mock.calls[0][1][0]).toMatchObject({
    error: expect.stringContaining("no longer available"),
  });
});

it("finishes an authorized page request when Chat closes over the same form", async () => {
  const { adapter, mode, events, rerender } = toolsSetup([]);
  mode.open = false;
  rerender({
    enabled: false,
    inputEvents: [
      events[0],
      event("late_request", "tool.call_requested", {
        tool_calls: [
          {
            id: "late",
            name: PAGE_PATCH_TOOL,
            arguments: { binding: "bound", revision: "v1", changes: { system_prompt: "Chat" } },
          },
        ],
      }),
    ],
  });
  await waitFor(() => expect(submitToolResults).toHaveBeenCalledTimes(1));
  expect(adapter.patch).toHaveBeenCalledTimes(1);
});

it("does not claim a page binding when mounted as full Chat", () => {
  const { adapter } = toolsSetup(undefined, false);
  expect(adapter.patch).not.toHaveBeenCalled();
  expect(submitToolResults).not.toHaveBeenCalled();
});
