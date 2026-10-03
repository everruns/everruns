import type React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { WORK_LOG_PAGE_SIZE } from "@/components/chat/turn-work-log";
import type { Event } from "@/lib/api/types";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => <div>{children}</div>,
}));
jest.mock("@streamdown/code", () => ({ code: {} }));
jest.mock("@streamdown/mermaid", () => ({ createMermaidPlugin: jest.fn(() => ({})) }));
jest.mock("remark-gfm", () => jest.fn());
jest.mock("remark-github-blockquote-alert", () => jest.fn());

jest.mock("@/hooks", () => ({
  useAgents: () => ({ data: [] }),
  useProviders: () => ({ data: [] }),
}));
jest.mock("@/hooks/use-members", () => ({ useMembers: () => ({ data: [] }) }));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({
    locale: "en",
    t: (key: string) => key,
  }),
}));

jest.mock("@/components/chat/work-log-narration", () => ({
  WorkLogNarration: ({ children }: { children: string }) => (
    <div data-testid="work-log-narration">{children}</div>
  ),
}));

jest.mock("@/components/chat/message-content", () => ({
  MessageContent: ({ text }: { text: string }) => <div>{text}</div>,
}));

jest.mock("@/components/chat/ask-user-tool-call", () => ({
  AskUserToolCall: () => <div>ask user card</div>,
  isAskUserArguments: () => true,
}));

jest.mock("@/components/chat/tool-activity-group", () => ({
  ToolActivityGroup: () => <div data-testid="tool-output" />,
}));

function event(id: string, type: string, data: Record<string, unknown>): Event {
  return {
    id,
    type,
    ts: `2026-08-08T04:00:0${id.length}Z`,
    session_id: "session-1",
    context: { turn_id: "turn-1" },
    data,
    sequence: id.length,
  };
}

function renderEmpty(emptyState?: React.ReactNode) {
  return render(
    <ChatMessageList
      events={[]}
      chatEvents={[]}
      sessionId="session-1"
      toolResultsMap={new Map()}
      toolProgressMap={new Map()}
      toolOutputMap={new Map()}
      eventsLoading={false}
      hasMoreEvents={false}
      loadingOlderEvents={false}
      getMessageText={() => ""}
      getToolCalls={() => []}
      emptyState={emptyState}
    />,
  );
}

describe("ChatMessageList empty state", () => {
  it("invites a first message by default", () => {
    renderEmpty();

    expect(screen.getByText("no_messages_yet")).toBeInTheDocument();
  });

  it("replaces that invitation with the override rather than stacking both", () => {
    renderEmpty(<div>Nothing to chat with</div>);

    expect(screen.getByText("Nothing to chat with")).toBeInTheDocument();
    expect(screen.queryByText("no_messages_yet")).not.toBeInTheDocument();
  });
});

describe("ChatMessageList work-log narration", () => {
  it("routes human reason.item and reason.completed text through the narration renderer", () => {
    const chatEvents = [
      event("tool", "tool.call_requested", {
        tool_calls: [{ id: "tool-1", name: "list_files", arguments: {} }],
      }),
      event("item", "reason.item", {
        turn_id: "turn-1",
        summary: ["Checked **configuration**"],
      }),
      event("done", "reason.completed", {
        success: true,
        text_preview: "Created [Hourly Dad Jokes](/agents/agent_123)",
        has_tool_calls: true,
        tool_call_count: 1,
      }),
    ];

    render(
      <ChatMessageList
        events={chatEvents}
        chatEvents={chatEvents}
        sessionId="session-1"
        toolResultsMap={new Map()}
        toolProgressMap={new Map()}
        toolOutputMap={new Map()}
        eventsLoading={false}
        hasMoreEvents={false}
        loadingOlderEvents={false}
        getMessageText={() => ""}
        getToolCalls={() => []}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /working/i }));

    expect(screen.getAllByTestId("work-log-narration")).toHaveLength(2);
    expect(screen.getByText("Checked **configuration**")).toBeInTheDocument();
    expect(screen.getByText("Created [Hourly Dad Jokes](/agents/agent_123)")).toBeInTheDocument();
  });
});

describe("ChatMessageList folded work log", () => {
  function renderEvents(chatEvents: Event[]) {
    return render(
      <ChatMessageList
        events={chatEvents}
        chatEvents={chatEvents}
        sessionId="session-1"
        toolResultsMap={new Map()}
        toolProgressMap={new Map()}
        toolOutputMap={new Map()}
        eventsLoading={false}
        hasMoreEvents={false}
        loadingOlderEvents={false}
        getMessageText={() => ""}
        getToolCalls={() => []}
      />,
    );
  }

  const actStarted = event("act", "act.started", {
    headline: "Creating the support agent",
    tool_calls: [
      { id: "tool-1", name: "create_agent", narration: "Creating the support agent" },
      { id: "tool-2", name: "get_agent", narration: "Verifying the support agent" },
    ],
  });
  const toolFailed = event("fail", "tool.completed", {
    tool_call_id: "tool-1",
    tool_name: "create_agent",
    success: false,
    status: "error",
    error: "schema error: required property 'system_prompt' is missing",
    result: [],
  });

  it("keeps a running turn folded, with its latest step and errors in the header", () => {
    renderEvents([actStarted, toolFailed]);

    expect(screen.getByRole("button", { name: /working/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByTestId("work-log-status")).toHaveTextContent("Creating the support agent");
    expect(screen.getByTestId("work-log-error-count")).toHaveTextContent("1 error");
  });

  it("shows interactive requests outside the fold while the turn runs", () => {
    renderEvents([
      actStarted,
      event("ask", "tool.call_requested", {
        tool_calls: [{ id: "ask-1", name: "ask_user", arguments: { questions: [] } }],
      }),
    ]);

    expect(screen.getByText("ask user card").closest("[aria-hidden='true']")).toBeNull();
  });

  it("keeps the error count once the turn completes", () => {
    renderEvents([
      actStarted,
      toolFailed,
      event("turn-completed", "turn.completed", { turn_id: "turn-1", duration_ms: 33000 }),
    ]);

    expect(screen.queryByTestId("work-log-status")).not.toBeInTheDocument();
    expect(screen.getByTestId("work-log-error-count")).toHaveTextContent("1 error");
  });
});

describe("ChatMessageList work log at scale", () => {
  // Structural budget for a turn with thousands of tool calls: folded, no tool
  // row is mounted; opened, only the newest page is. Rendering every hidden
  // row made each streamed event cost about a second at 3,000 calls.
  it("keeps a 5,000-call turn cheap while folded and when opened", () => {
    const callCount = 5000;
    const chatEvents: Event[] = [];
    for (let index = 0; index < callCount; index += 1) {
      const context = { turn_id: "turn-1", exec_id: `exec-${index}` };
      chatEvents.push(
        {
          ...event(`act-${index}`, "act.started", {
            headline: `Editing module ${index}`,
            tool_calls: [
              { id: `call-${index}`, name: "edit_file", narration: `Editing module ${index}` },
            ],
          }),
          context,
        },
        {
          ...event(`done-${index}`, "tool.completed", {
            tool_call_id: `call-${index}`,
            tool_name: "edit_file",
            success: true,
            status: "success",
            result: [{ type: "text", text: "ok" }],
          }),
          context,
        },
      );
    }

    render(
      <ChatMessageList
        events={chatEvents}
        chatEvents={chatEvents}
        sessionId="session-1"
        toolResultsMap={new Map()}
        toolProgressMap={new Map()}
        toolOutputMap={new Map()}
        eventsLoading={false}
        hasMoreEvents={false}
        loadingOlderEvents={false}
        getMessageText={() => ""}
        getToolCalls={() => []}
      />,
    );

    // Only the header's status line mentions a step while folded.
    expect(screen.getAllByText(/^Editing module/)).toHaveLength(1);
    expect(screen.getByTestId("work-log-status")).toHaveTextContent("Editing module 4999");

    fireEvent.click(screen.getByRole("button", { name: /working/i }));

    // The newest page of rows, plus the status line.
    expect(screen.getAllByText(/^Editing module/)).toHaveLength(WORK_LOG_PAGE_SIZE + 1);
    expect(screen.queryByText("Editing module 0")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "show_earlier_steps" })).toBeInTheDocument();
  });
});
