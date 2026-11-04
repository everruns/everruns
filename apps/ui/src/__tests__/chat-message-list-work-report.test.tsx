import { render, screen } from "@testing-library/react";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import type { Event, WorkReport } from "@/lib/api/types";

const mockUseTurnWorkReport = jest.fn();
jest.mock("@/hooks/use-turn-work-report", () => ({
  useTurnWorkReport: (...args: unknown[]) => mockUseTurnWorkReport(...args),
}));

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
  ...jest.requireActual("@/providers/locale-provider"),
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

const turnStarted = event("start", "turn.started", { turn_id: "turn-1", input_message_id: "m1" });
const actStarted = event("act", "act.started", {
  headline: "Running tests",
  tool_calls: [{ id: "tool-1", name: "bash", narration: "Running `cargo test`" }],
});

function workingReport(): WorkReport {
  return {
    turn_id: "turn-1",
    state: "working",
    headline: "Running `cargo test` 7 of 12",
    steps: [
      {
        id: "tool-1",
        tool_name: "bash",
        label: "Running `cargo test`",
        state: "running",
        started_at: "2026-08-08T04:00:05Z",
        progress: { current: 7, total: 12 },
      },
    ],
    plan: {
      items: [
        { title: "Fix the rounding", status: "done" },
        { title: "Run the tests", status: "in_progress" },
      ],
      done: 1,
      total: 2,
      current: "Running the tests",
    },
  } as WorkReport;
}

beforeEach(() => mockUseTurnWorkReport.mockReset());

describe("ChatMessageList work report", () => {
  it("reads the latest turn's report: live headline in the header, plan pinned", () => {
    mockUseTurnWorkReport.mockReturnValue(workingReport());
    renderEvents([turnStarted, actStarted]);

    expect(mockUseTurnWorkReport).toHaveBeenCalledWith("session-1", "turn-1", expect.any(String));
    expect(screen.getByTestId("work-log-status")).toHaveTextContent("Running `cargo test` 7 of 12");
    expect(screen.getByTestId("work-plan")).toHaveTextContent("Run the tests");
  });

  it("shows what got done under the header once the turn ends", () => {
    mockUseTurnWorkReport.mockReturnValue({
      ...workingReport(),
      state: "done",
      headline: "Done in 13s · 2 files changed",
      plan: undefined,
      result: {
        summary: "Fixed proration rounding.",
        summary_source: "agent",
        facts: [{ kind: "files_changed", value: 2 }],
      },
    } as WorkReport);
    renderEvents([
      turnStarted,
      actStarted,
      event("turn-completed", "turn.completed", {
        turn_id: "turn-1",
        duration_ms: 13000,
        iterations: 2,
      }),
    ]);

    expect(screen.queryByTestId("work-plan")).not.toBeInTheDocument();
    expect(screen.getByTestId("work-result")).toHaveTextContent("Fixed proration rounding.");
  });

  it("ignores a report for another turn", () => {
    mockUseTurnWorkReport.mockReturnValue({ ...workingReport(), turn_id: "turn-0" } as WorkReport);
    renderEvents([turnStarted, actStarted]);

    expect(screen.queryByTestId("work-plan")).not.toBeInTheDocument();
  });
});
