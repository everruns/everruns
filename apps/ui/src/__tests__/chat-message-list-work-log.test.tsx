import { render, screen } from "@testing-library/react";
import { ChatMessageList } from "@/components/chat/chat-message-list";
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

    expect(screen.getAllByTestId("work-log-narration")).toHaveLength(2);
    expect(screen.getByText("Checked **configuration**")).toBeInTheDocument();
    expect(screen.getByText("Created [Hourly Dad Jokes](/agents/agent_123)")).toBeInTheDocument();
  });
});
