import { render, screen } from "@testing-library/react";
import type { Event } from "@/lib/api/types";

function event(type: string, sequence: number, text: string): Event {
  return {
    id: `event_${sequence}`,
    session_id: "session_123",
    type,
    sequence,
    ts: "2026-08-10T12:00:00Z",
    data: {
      message: { content: [{ type: "text", text }] },
    },
  } as Event;
}

const completedEvents = [
  event("input.message", 1, "What happened?"),
  event("output.message.completed", 2, "The run completed."),
];

const mockContext = {
  events: completedEvents as Event[],
  sessionId: "session_123",
  chatEvents: completedEvents as Event[],
  toolResultsMap: new Map(),
  toolProgressMap: new Map(),
  toolOutputMap: new Map(),
  eventsLoading: false,
  isThinking: false,
  streamingText: null as string | null,
  streamingMessageId: null as string | null,
  streamingIteration: null as number | null,
  hasMoreEvents: false,
  loadingOlderEvents: false,
  loadOlderEvents: jest.fn(),
  getMessageText: (data: { message?: { content?: Array<{ text?: string }> } }) =>
    data.message?.content?.[0]?.text ?? "",
  getToolCalls: () => [],
};

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => mockContext,
}));

jest.mock("@/hooks", () => ({
  useMessageScrollerVisibility: () => ({ currentAnchorId: null, scrollToAnchor: jest.fn() }),
  useScrollManager: () => ({
    scrollContainerRef: { current: null },
    messagesEndRef: { current: null },
    hasNewMessages: false,
    dismissNewMessages: jest.fn(),
    handleScrollUp: jest.fn(),
  }),
  useSessionParticipants: () => ({ data: [] }),
  useTurnKeyboardNavigation: jest.fn(),
}));

jest.mock("@/hooks/use-thread-runs", () => ({
  useThreadRuns: () => [],
}));

jest.mock("@/components/chat/chat-message-list", () => ({
  ChatMessageList: ({ chatEvents }: { chatEvents: Event[] }) => (
    <div data-testid="completed-transcript">{chatEvents.map((item) => item.type).join(",")}</div>
  ),
}));

jest.mock("@/components/chat/chat-nav-rail", () => ({
  ChatNavRail: () => <nav aria-label="Conversation turns" />,
}));

jest.mock("@/components/streaming-message", () => ({
  StreamingMessage: ({ text }: { text: string }) => <div data-testid="streaming-text">{text}</div>,
}));

jest.mock("@/components/thinking-indicator", () => ({
  ThinkingIndicator: () => <div>Thinking</div>,
}));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ t: (key: string) => key }),
}));

import { SessionTranscript } from "@/components/session/session-transcript";

describe("SessionTranscript replay and streaming", () => {
  beforeEach(() => {
    mockContext.events = completedEvents;
    mockContext.chatEvents = completedEvents;
    mockContext.isThinking = false;
    mockContext.streamingText = null;
    mockContext.streamingMessageId = null;
    mockContext.streamingIteration = null;
  });

  it("replays the completed human-readable conversation", () => {
    render(<SessionTranscript />);

    expect(screen.getByTestId("completed-transcript")).toHaveTextContent(
      "input.message,output.message.completed",
    );
    expect(screen.queryByTestId("streaming-text")).not.toBeInTheDocument();
  });

  it("shows live output and settles back to completed replay", () => {
    mockContext.events = [completedEvents[0]];
    mockContext.chatEvents = [completedEvents[0]];
    mockContext.isThinking = true;
    mockContext.streamingText = "The run is still";
    mockContext.streamingMessageId = "message_live";

    const { rerender } = render(<SessionTranscript />);
    expect(screen.getByTestId("streaming-text")).toHaveTextContent("The run is still");

    mockContext.events = completedEvents;
    mockContext.chatEvents = completedEvents;
    mockContext.isThinking = false;
    mockContext.streamingText = null;
    mockContext.streamingMessageId = null;
    rerender(<SessionTranscript />);

    expect(screen.queryByTestId("streaming-text")).not.toBeInTheDocument();
    expect(screen.getByTestId("completed-transcript")).toHaveTextContent(
      "output.message.completed",
    );
  });
});
