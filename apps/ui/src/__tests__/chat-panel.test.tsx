import { render, screen } from "@testing-library/react";
import React from "react";
import { ChatPanel } from "@/components/chat/chat-panel";

const mockUseSessionCommands = jest.fn();

const mockSessionContext = {
  agentId: "agent-1",
  events: [],
  sessionId: "session-1",
  llmModel: null,
  chatEvents: [],
  toolResultsMap: new Map(),
  eventsLoading: false,
  isActive: false,
  reasoningEffort: "",
  setReasoningEffort: jest.fn(),
  setIsWaitingForResponse: jest.fn(),
  isThinking: false,
  streamingText: "",
  streamingIteration: 0,
  sendMessage: {
    mutateAsync: jest.fn(),
    isPending: false,
  },
  cancelCurrentTurn: jest.fn(),
  hasMoreEvents: false,
  loadingOlderEvents: false,
  loadOlderEvents: jest.fn(),
  getMessageText: jest.fn(() => ""),
  getToolCalls: jest.fn(() => []),
};

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => mockSessionContext,
}));

jest.mock("@/hooks", () => ({
  useLlmModels: () => ({ data: [] }),
  useImageAttachments: () => ({
    pendingImages: [],
    allUploaded: true,
    uploadedImageIds: [],
    addFiles: jest.fn(),
    removeImage: jest.fn(),
    clearImages: jest.fn(),
    hasImages: false,
    isUploading: false,
  }),
  useSessionCommands: (...args: unknown[]) => mockUseSessionCommands(...args),
  useScrollManager: () => ({
    scrollContainerRef: { current: null },
    messagesEndRef: { current: null },
    hasNewMessages: false,
    dismissNewMessages: jest.fn(),
    handleScrollUp: jest.fn(),
    scrollToBottom: jest.fn(),
  }),
  useImageDropZone: () => ({
    isDraggingOver: false,
    dropZoneProps: {
      onDragOver: jest.fn(),
      onDragEnter: jest.fn(),
      onDragLeave: jest.fn(),
      onDrop: jest.fn(),
    },
    handlePaste: jest.fn(),
  }),
}));

jest.mock("@tanstack/react-query", () => ({
  useMutation: () => ({
    mutateAsync: jest.fn(),
    isPending: false,
  }),
}));

jest.mock("@/lib/api/messages", () => ({
  sendUserMessageWithImages: jest.fn(),
}));

jest.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: React.ButtonHTMLAttributes<HTMLButtonElement> & { children?: React.ReactNode }) => (
    <button {...props}>{children}</button>
  ),
}));

jest.mock("@/components/ui/skeleton", () => ({
  Skeleton: ({ className }: { className?: string }) => <div className={className} />,
}));

jest.mock("@/components/ui/textarea", () => ({
  Textarea: React.forwardRef<
    HTMLTextAreaElement,
    React.TextareaHTMLAttributes<HTMLTextAreaElement>
  >(function MockTextarea(props, ref) {
    return <textarea ref={ref} {...props} />;
  }),
}));

jest.mock("@/components/ui/select", () => ({
  Select: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectItem: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectTrigger: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectValue: ({ children }: { children?: React.ReactNode }) => <div>{children}</div>,
}));

jest.mock("@/components/chat/message-info-icon", () => ({
  MessageInfoIcon: () => null,
}));

jest.mock("@/components/chat/image-attachments", () => ({
  ImageAttachments: () => null,
  MessageImage: () => null,
}));

jest.mock("@/components/chat/command-autocomplete", () => ({
  CommandAutocomplete: () => null,
  shouldShowCommandAutocomplete: (input: string) => /^\/\S*$/.test(input),
}));

jest.mock("@/components/thinking-indicator", () => ({
  ThinkingIndicator: () => null,
}));

jest.mock("@/components/streaming-message", () => ({
  StreamingMessage: () => null,
}));

jest.mock("@/components/chat/message-content", () => ({
  MessageContent: ({ text }: { text: string }) => <div>{text}</div>,
}));

jest.mock("@/components/chat/tool-activity-group", () => ({
  ToolActivityGroup: () => null,
}));

jest.mock("@/components/chat/tool-activity-timeline-group", () => ({
  ToolActivityTimelineGroup: () => null,
}));

jest.mock("@/components/chat/setup-connection-tool-call", () => ({
  SetupConnectionToolCall: () => null,
}));

beforeAll(() => {
  Object.defineProperty(window.HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: jest.fn(),
  });
});

describe("ChatPanel compaction divider", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });
  });

  it("renders compaction divider with message counts and strategy", () => {
    const compactedEvent = {
      id: "evt-compacted-1",
      type: "context.compacted",
      session_id: "session-1",
      turn_id: "turn-1",
      created_at: new Date().toISOString(),
      data: {
        strategy_used: "observation_masking",
        messages_before: 42,
        messages_after: 28,
        duration_ms: 150,
        steps: [
          {
            strategy: "observation_masking",
            messages_after: 28,
            duration_ms: 150,
          },
        ],
      },
    };
    mockSessionContext.chatEvents = [compactedEvent];

    render(<ChatPanel />);

    expect(screen.getByText(/Context compacted/)).toBeInTheDocument();
    expect(screen.getByText(/42 → 28 messages/)).toBeInTheDocument();
    expect(screen.getByText(/observation_masking/)).toBeInTheDocument();
  });

  it("renders compaction divider without strategy when strategy is none", () => {
    const compactedEvent = {
      id: "evt-compacted-2",
      type: "context.compacted",
      session_id: "session-1",
      turn_id: "turn-1",
      created_at: new Date().toISOString(),
      data: {
        strategy_used: "none",
        messages_before: 20,
        messages_after: 20,
        duration_ms: 0,
        steps: [],
      },
    };
    mockSessionContext.chatEvents = [compactedEvent];

    render(<ChatPanel />);

    expect(screen.getByText(/Context compacted/)).toBeInTheDocument();
    expect(screen.getByText(/20 → 20 messages/)).toBeInTheDocument();
    // Should NOT show "none" as a strategy
    expect(screen.queryByText(/· none/)).not.toBeInTheDocument();
  });

  it("renders compaction divider with auto strategy showing multiple steps", () => {
    const compactedEvent = {
      id: "evt-compacted-3",
      type: "context.compacted",
      session_id: "session-1",
      turn_id: "turn-1",
      created_at: new Date().toISOString(),
      data: {
        strategy_used: "auto",
        messages_before: 100,
        messages_after: 30,
        duration_ms: 2500,
        steps: [
          { strategy: "observation_masking", messages_after: 70, duration_ms: 50 },
          { strategy: "summarization", messages_after: 30, duration_ms: 2450 },
        ],
      },
    };
    mockSessionContext.chatEvents = [compactedEvent];

    render(<ChatPanel />);

    expect(screen.getByText(/Context compacted/)).toBeInTheDocument();
    expect(screen.getByText(/100 → 30 messages/)).toBeInTheDocument();
    expect(screen.getByText(/auto/)).toBeInTheDocument();
  });
});

describe("ChatPanel placeholder", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("does not advertise slash commands when none are available", () => {
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });

    render(<ChatPanel />);

    expect(screen.getByPlaceholderText("Type a message... (Enter to send)")).toBeInTheDocument();
    expect(
      screen.queryByPlaceholderText("Type a message or / for commands... (Enter to send)"),
    ).not.toBeInTheDocument();
  });

  it("keeps slash command hint when commands are available", () => {
    mockUseSessionCommands.mockReturnValue({
      data: {
        commands: [
          {
            name: "clear",
            description: "Clear the session",
            source: "system",
          },
        ],
      },
    });

    render(<ChatPanel />);

    expect(
      screen.getByPlaceholderText("Type a message or / for commands... (Enter to send)"),
    ).toBeInTheDocument();
  });

  it("does not render a top border between messages and composer", () => {
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });

    render(<ChatPanel />);

    const composer = screen.getByPlaceholderText("Type a message... (Enter to send)");
    const composerShell = composer.closest("form")?.parentElement;

    expect(composerShell).not.toBeNull();
    expect(composerShell).not.toHaveClass("border-t");
  });
});
