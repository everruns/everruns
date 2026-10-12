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
  it.each([false, true])(
    "does not repeat assistant output as reasoning (collapseWorkLog: %s)",
    (collapseWorkLog) => {
      const answer = "I can help with the code in this workspace, including exploring the project.";
      const preview = answer.slice(0, 60);
      const chatEvents = [
        event("item", "reason.item", {
          turn_id: "turn-1",
          summary: ["Checked the available workspace capabilities"],
        }),
        event("message", "output.message.completed", {
          message: { content: [{ type: "text", text: answer }] },
        }),
        event("done", "reason.completed", {
          success: true,
          text_preview: preview,
          has_tool_calls: false,
          tool_call_count: 0,
        }),
        event("end", "turn.completed", {
          turn_id: "turn-1",
          duration_ms: 9000,
          iterations: 2,
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
          getMessageText={(data) =>
            data.message?.content
              ?.flatMap((part) => (part.type === "text" ? [part.text] : []))
              .join("") ?? ""
          }
          getToolCalls={() => []}
          collapseWorkLog={collapseWorkLog}
        />,
      );

      if (collapseWorkLog) {
        fireEvent.click(screen.getByRole("button", { name: /worked_for/i }));
      }
      expect(screen.getAllByText(answer)).toHaveLength(1);
      expect(screen.queryByText(preview, { exact: true })).not.toBeInTheDocument();
      expect(screen.getByText("Checked the available workspace capabilities")).toBeInTheDocument();
    },
  );

  it("renders provider reasoning summaries without assistant output previews", () => {
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

    expect(screen.getAllByTestId("work-log-narration")).toHaveLength(1);
    expect(screen.getByText("Checked **configuration**")).toBeInTheDocument();
    expect(
      screen.queryByText("Created [Hourly Dad Jokes](/agents/agent_123)"),
    ).not.toBeInTheDocument();
  });

  it("folds commentary into the work log and leaves the final answer as a message", () => {
    const chatEvents = [
      event("tool", "tool.call_requested", {
        tool_calls: [{ id: "tool-1", name: "list_files", arguments: {} }],
      }),
      event("note", "output.message.completed", {
        message: {
          id: "message-note",
          role: "agent",
          phase: "commentary",
          phase_source: "provider",
          content: [
            {
              type: "text",
              text: "There isn't a registered GitHub MCP server yet. I'll add it.",
            },
          ],
        },
      }),
      event("answer", "output.message.completed", {
        message: {
          id: "message-answer",
          role: "agent",
          phase: "final_answer",
          content: [{ type: "text", text: "GitHub is ready to authorise." }],
        },
      }),
      event("done", "turn.completed", { turn_id: "turn-1", duration_ms: 111000 }),
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
        getMessageText={(data) =>
          data.message?.content
            ?.flatMap((part) => (part.type === "text" ? [part.text] : []))
            .join("") ?? ""
        }
        getToolCalls={() => []}
      />,
    );

    expect(screen.getByText("GitHub is ready to authorise.")).toBeInTheDocument();
    expect(
      screen.queryByText("There isn't a registered GitHub MCP server yet. I'll add it."),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /worked_for/i }));

    expect(
      screen.getByText("There isn't a registered GitHub MCP server yet. I'll add it."),
    ).toBeInTheDocument();
  });

  it("shows a sent conversation.message as an agent message and keeps explicit notes in the work log", () => {
    const note = "Checking the deploy log before answering.";
    const reply = "The deploy **finished** at 10:02.";
    const chatEvents = [
      event("note", "output.message.completed", {
        message: {
          id: "message-note",
          role: "agent",
          phase: "commentary",
          phase_source: "communication",
          content: [
            { type: "text", text: note },
            {
              type: "tool_call",
              id: "call-send",
              name: "send_message",
              arguments: { text: reply },
            },
          ],
        },
      }),
      event("sent", "conversation.message", {
        message_id: "message_sent_1",
        text: reply,
        tool_call_id: "call-send",
      }),
      event("done", "turn.completed", { turn_id: "turn-1", duration_ms: 4000, iterations: 2 }),
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
        getMessageText={(data) =>
          data.message?.content
            ?.flatMap((part) => (part.type === "text" ? [part.text] : []))
            .join("") ?? ""
        }
        getToolCalls={(data) =>
          (data.message?.content ?? []).flatMap((part) =>
            part.type === "tool_call"
              ? [{ id: part.id, name: part.name, arguments: part.arguments }]
              : [],
          )
        }
      />,
    );

    // The reply is a normal agent bubble, rendered once.
    expect(screen.getAllByText(reply)).toHaveLength(1);
    expect(screen.getByText(reply).closest("[data-conversation-message-id]")).toHaveAttribute(
      "data-conversation-message-id",
      "message_sent_1",
    );
    // Notes and the send_message call stay folded in the work log.
    expect(screen.queryByText(note)).not.toBeInTheDocument();
    expect(screen.queryByTestId("tool-output")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /worked_for/i }));

    expect(screen.getByTestId("work-log-narration")).toHaveTextContent(note);
    expect(screen.getByTestId("tool-output")).toBeInTheDocument();
    expect(screen.getAllByText(reply)).toHaveLength(1);
  });

  it("skips an empty conversation.message", () => {
    const chatEvents = [
      event("sent", "conversation.message", {
        message_id: "message_sent_1",
        text: "   ",
        tool_call_id: "call-send",
      }),
    ];
    const { container } = render(
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
    expect(container.querySelector("[data-conversation-message-id]")).toBeNull();
  });

  it("shows live thinking inside Working instead of an assistant message", () => {
    render(
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
        streamingWork={{ turnId: "turn-1", text: null, isThinking: true }}
      />,
    );

    expect(screen.queryByText("no_messages_yet")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /working/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );

    fireEvent.click(screen.getByRole("button", { name: /working/i }));

    expect(screen.getByText("thinking")).toBeInTheDocument();
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

describe("ChatMessageList full work log", () => {
  function renderFullLog(chatEvents: Event[]) {
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
        getMessageText={(data) =>
          data.message?.content
            ?.flatMap((part) => (part.type === "text" ? [part.text] : []))
            .join("") ?? ""
        }
        getToolCalls={(data) =>
          data.message?.content?.flatMap((part) => (part.type === "tool_call" ? [part] : [])) ?? []
        }
        collapseWorkLog={false}
      />,
    );
  }

  it.each([false, true])(
    "shows all activity and errors without a turn fold (completed: %s)",
    (completed) => {
      const events = [
        event("act", "act.started", {
          headline: "Updating the agent",
          tool_calls: [
            { id: "tool-1", name: "create_agent", narration: "Creating the agent" },
            { id: "tool-2", name: "get_agent", narration: "Checking the agent" },
          ],
        }),
        event("failed", "tool.completed", {
          tool_call_id: "tool-1",
          tool_name: "create_agent",
          success: false,
          status: "error",
          error: "Missing system prompt",
          result: [],
        }),
        event("checked", "tool.completed", {
          tool_call_id: "tool-2",
          tool_name: "get_agent",
          success: true,
          status: "success",
          result: [],
        }),
        event("ask", "tool.call_requested", {
          tool_calls: [{ id: "ask-1", name: "ask_user", arguments: { questions: [] } }],
        }),
        ...(completed
          ? [event("end", "turn.completed", { turn_id: "turn-1", duration_ms: 9000 })]
          : []),
      ];
      renderFullLog(events);

      expect(screen.queryByRole("button", { name: /working|worked_for/i })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /activity_group/i })).not.toBeInTheDocument();
      for (const text of [
        "Creating the agent",
        "Checking the agent",
        "Missing system prompt",
        "ask user card",
      ]) {
        expect(screen.getByText(text).closest("[aria-hidden='true']")).toBeNull();
      }
    },
  );

  it("keeps reasoning in event order around intermediate messages, even for a single iteration", () => {
    renderFullLog([
      event("first", "reason.item", { turn_id: "turn-1", summary: ["First step"] }),
      event("message", "output.message.completed", {
        message: { content: [{ type: "text", text: "Intermediate message" }] },
      }),
      event("second", "reason.item", { turn_id: "turn-1", summary: ["Second step"] }),
      event("end", "turn.completed", { turn_id: "turn-1", duration_ms: 9000, iterations: 1 }),
    ]);

    const first = screen.getByText("First step");
    const message = screen.getByText("Intermediate message");
    const second = screen.getByText("Second step");
    expect(first.compareDocumentPosition(message) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(message.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("does not page away earlier work entries", () => {
    const events = Array.from({ length: WORK_LOG_PAGE_SIZE + 1 }, (_, index) =>
      event(`reason-${index}`, "reason.item", { turn_id: "turn-1", summary: [`Step ${index}`] }),
    );
    renderFullLog(events);

    expect(screen.getAllByTestId("work-log-narration")).toHaveLength(WORK_LOG_PAGE_SIZE + 1);
    expect(screen.getByText("Step 0")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "show_earlier_steps" })).not.toBeInTheDocument();
  });

  it("shows tool-only assistant messages without a fold, including uncorrelated events", () => {
    renderFullLog([
      {
        ...event("tools", "output.message.completed", {
          message: {
            content: [{ type: "tool_call", id: "tool-1", name: "list_files", arguments: {} }],
          },
        }),
        context: {},
      },
    ]);

    expect(screen.getByTestId("tool-output")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /working|worked_for/i })).not.toBeInTheDocument();
  });
});

describe("ChatMessageList platform messages", () => {
  function renderMessages(messages: { text: string; metadata?: Record<string, string> }[]) {
    const chatEvents = messages.map(({ text, metadata }, index) =>
      event("m".repeat(index + 1), "input.message", {
        message: { role: "user", content: [{ type: "text", text }], metadata },
      }),
    );
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
        getMessageText={(data) =>
          data.message?.content
            ?.flatMap((part) => (part.type === "text" ? [part.text] : []))
            .join("") ?? ""
        }
        getToolCalls={() => []}
      />,
    );
  }

  it("shows an automatic update without the task id", () => {
    renderMessages([
      {
        text: 'Task "Releases" (task_01abc) finished: succeeded.\n- summary: Found v0.44.0.',
        metadata: { everruns_origin: "task_wake" },
      },
    ]);
    expect(screen.getByText("automatic_update")).toBeInTheDocument();
    expect(screen.getByText("task_update_done")).toBeInTheDocument();
    expect(screen.getByText("Found v0.44.0.")).toBeInTheDocument();
    expect(screen.queryByText(/task_01abc/)).not.toBeInTheDocument();
  });

  it("shows a coordinator brief without the worker's instructions", () => {
    renderMessages([
      {
        text: "New assignment from the coordinator: Releases\n\nList the releases.\n\nKeep your checklist current with update_checklist. Then finish.",
      },
    ]);
    expect(screen.getByText("from_chat")).toBeInTheDocument();
    expect(screen.getByText("List the releases.")).toBeInTheDocument();
    expect(screen.queryByText(/update_checklist/)).not.toBeInTheDocument();
  });

  it("shows what the person typed as written", () => {
    renderMessages([{ text: 'Task "x" is what I typed' }]);
    expect(screen.getByText('Task "x" is what I typed')).toBeInTheDocument();
    expect(screen.queryByText("from_chat")).not.toBeInTheDocument();
  });
});
