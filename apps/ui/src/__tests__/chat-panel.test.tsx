import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";
import { ChatPanel } from "@/components/chat/chat-panel";
import { ApiError } from "@/lib/api/client";
import type { Event, ModelWithProvider } from "@/lib/api/types";

const mockUseSessionCommands = jest.fn();
const mockExecuteSessionCommand = jest.fn();
const mockUseFeatureFlag = jest.fn((..._args: unknown[]) => false);
const mockStartSessionVoice = jest.fn();
const mockEndSessionVoice = jest.fn();
const mockModelEffortMenu = jest.fn((_props: Record<string, unknown>): React.ReactNode => null);
const mockParticipants: Array<Record<string, unknown>> = [];
const mockAgents: Array<Record<string, unknown>> = [];
const mockModels: ModelWithProvider[] = [];

const availableDefaultModel: ModelWithProvider = {
  id: "model-default",
  provider_id: "provider-1",
  model_id: "gpt-5.4",
  display_name: "GPT-5.4",
  capabilities: ["chat"],
  enabled: true,
  healthy: true,
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
  provider_name: "OpenAI",
  provider_type: "openai",
  is_favorite: false,
  profile: {
    name: "GPT-5.4",
    family: "gpt-5",
    attachment: false,
    reasoning: false,
    temperature: true,
    tool_call: true,
    structured_output: true,
    open_weights: false,
  },
};

const mockSessionContext = {
  agentId: "agent-1",
  events: [],
  sessionId: "session-1",
  llmModel: availableDefaultModel as ModelWithProvider | null,
  llmModelLoading: false,
  chatEvents: [] as Event[],
  toolResultsMap: new Map(),
  toolProgressMap: new Map(),
  toolOutputMap: new Map(),
  eventsLoading: false,
  isActive: false,
  reasoningEffort: "",
  setReasoningEffort: jest.fn(),
  verbosity: "",
  setVerbosity: jest.fn(),
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

// SessionTaskChips pulls in org context + an SSE subscription; this suite
// tests chat behavior, not the task strip.
// Run cards are the Chats thread surface's concern and are covered by
// run-cards.test.ts; this suite renders ChatPanel outside an OrgProvider.
jest.mock("@/hooks/use-thread-runs", () => ({
  useThreadRuns: () => [],
}));

jest.mock("@/components/session/session-task-chips", () => ({
  SessionTaskChips: () => null,
}));

// The participants rail pulls in org context + participant queries; this suite
// tests chat behavior, not the rail.
jest.mock("@/components/session/session-participants-rail", () => ({
  SessionParticipantsRail: () => null,
}));

jest.mock("@/hooks", () => ({
  useModels: () => ({ data: mockModels, isLoading: false }),
  useProviders: () => ({ data: [] }),
  useAgents: () => ({ data: mockAgents }),
  useSessionParticipants: () => ({ data: mockParticipants }),
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
  useMessageScrollerVisibility: () => ({
    currentAnchorId: null,
    visibleAnchorIds: [],
    scrollToAnchor: jest.fn(),
  }),
  useTurnKeyboardNavigation: () => undefined,
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
  useMutation: ({ mutationFn }: { mutationFn: (...args: unknown[]) => unknown }) => ({
    mutateAsync: mutationFn,
    isPending: false,
  }),
}));

jest.mock("@/lib/api/commands", () => ({
  executeSessionCommand: (...args: unknown[]) => mockExecuteSessionCommand(...args),
}));

jest.mock("@/lib/api/messages", () => ({
  sendUserMessageWithImages: jest.fn(),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (...args: unknown[]) => mockUseFeatureFlag(...args),
}));

jest.mock("@/lib/api/voice", () => ({
  startSessionVoice: (...args: unknown[]) => mockStartSessionVoice(...args),
  endSessionVoice: (...args: unknown[]) => mockEndSessionVoice(...args),
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

jest.mock("@/components/chat/model-effort-menu", () => ({
  ModelEffortMenu: (props: Record<string, unknown>) => mockModelEffortMenu(props),
}));

jest.mock("@/components/ui/dialog", () => ({
  Dialog: ({
    children,
    open,
  }: {
    children: React.ReactNode;
    open?: boolean;
    onOpenChange?: (open: boolean) => void;
  }) => (open ? <div>{children}</div> : null),
  DialogContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DialogDescription: ({ children }: { children?: React.ReactNode }) => <div>{children}</div>,
  DialogFooter: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DialogHeader: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  DialogTitle: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

jest.mock("@/components/chat/message-info-icon", () => ({
  MessageInfoIcon: () => null,
}));

jest.mock("@/components/chat/image-attachments", () => ({
  ImageAttachments: () => null,
  MessageImage: () => null,
}));

jest.mock("@/components/chat/command-autocomplete", () => ({
  CommandAutocomplete: ({
    commands,
    visible,
    onSelect,
  }: {
    commands: Array<{ name: string }>;
    visible: boolean;
    onSelect: (command: { name: string }) => void;
  }) =>
    visible ? (
      <div>
        {commands.map((command) => (
          <button key={command.name} type="button" onClick={() => onSelect(command)}>
            /{command.name}
          </button>
        ))}
      </div>
    ) : null,
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

jest.mock("@/components/chat/work-log-narration", () => ({
  WorkLogNarration: ({ children }: { children: string }) => <div>{children}</div>,
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

beforeEach(() => {
  window.localStorage.clear();
  mockUseFeatureFlag.mockReturnValue(false);
  mockStartSessionVoice.mockReset();
  mockEndSessionVoice.mockReset();
  mockParticipants.length = 0;
  mockAgents.length = 0;
  mockModels.length = 0;
  mockSessionContext.llmModelLoading = false;
});

describe("ChatPanel compaction divider", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockModelEffortMenu.mockClear();
    mockExecuteSessionCommand.mockReset();
    mockSessionContext.chatEvents = [];
    mockSessionContext.llmModel = availableDefaultModel;
    mockSessionContext.reasoningEffort = "";
    mockSessionContext.sessionId = "session-1";
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });
  });

  it("renders compaction divider with message counts and strategy", () => {
    const compactedEvent = {
      id: "evt-compacted-1",
      type: "context.compacted",
      session_id: "session-1",
      ts: new Date().toISOString(),
      context: { turn_id: "turn-1" },
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
      ts: new Date().toISOString(),
      context: { turn_id: "turn-1" },
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
      ts: new Date().toISOString(),
      context: { turn_id: "turn-1" },
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
    mockExecuteSessionCommand.mockReset();
    mockSessionContext.chatEvents = [];
    mockSessionContext.llmModel = availableDefaultModel;
    mockSessionContext.reasoningEffort = "";
    mockSessionContext.sendMessage.mutateAsync.mockReset();
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });
  });

  it("does not advertise slash commands when none are available", () => {
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });

    render(<ChatPanel />);

    expect(screen.getByPlaceholderText("Type a message... (Enter to send)")).toBeInTheDocument();
    expect(
      screen.queryByPlaceholderText("Type a message or / for commands... (Enter to send)"),
    ).not.toBeInTheDocument();
  });

  it("does not submit while the default model is unavailable", async () => {
    mockSessionContext.llmModel = null;
    mockSessionContext.llmModelLoading = false;

    render(<ChatPanel />);

    const textarea = screen.getByPlaceholderText("Type a message... (Enter to send)");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    expect(mockSessionContext.sendMessage.mutateAsync).not.toHaveBeenCalled();
    expect(screen.getByText(/Choose a model/)).toBeInTheDocument();
  });

  it("waits for default model resolution before enabling submission", () => {
    mockSessionContext.llmModel = null;
    mockSessionContext.llmModelLoading = true;

    const { container } = render(<ChatPanel />);

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "hello" } });

    expect(screen.getByText("Checking model availability…")).toBeInTheDocument();
    expect(container.querySelector('button[type="submit"]')).toBeDisabled();
  });

  it("sends an explicit model when no default is available", async () => {
    mockSessionContext.llmModel = null;
    mockModels.push({ ...availableDefaultModel, id: "model-explicit" });
    mockModelEffortMenu.mockImplementationOnce((props: Record<string, unknown>) => (
      <button
        type="button"
        onClick={() => (props.onModelChange as (id: string) => void)("model-explicit")}
      >
        Choose explicit model
      </button>
    ));
    mockSessionContext.sendMessage.mutateAsync.mockResolvedValueOnce(undefined);

    render(<ChatPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Choose explicit model" }));
    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    await waitFor(() =>
      expect(mockSessionContext.sendMessage.mutateAsync).toHaveBeenCalledWith({
        sessionId: "session-1",
        content: "hello",
        controls: { locale: "en-US", model_id: "model-explicit" },
        addressedParticipantId: null,
      }),
    );
  });

  it("does not submit a stale explicit model that is no longer enabled", async () => {
    mockSessionContext.llmModel = null;
    mockModels.push({ ...availableDefaultModel, id: "model-disabled", enabled: false });
    window.localStorage.setItem(
      "everruns:chat:model-selection:agent-1:session-1",
      "model-disabled",
    );

    render(<ChatPanel />);

    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    expect(mockSessionContext.sendMessage.mutateAsync).not.toHaveBeenCalled();
    expect(screen.getByText(/Choose a model/)).toBeInTheDocument();
  });

  it("inherits a resolved default without adding a message override", async () => {
    mockSessionContext.llmModel = availableDefaultModel;
    mockSessionContext.sendMessage.mutateAsync.mockResolvedValueOnce(undefined);

    render(<ChatPanel />);
    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    await waitFor(() =>
      expect(mockSessionContext.sendMessage.mutateAsync).toHaveBeenCalledWith({
        sessionId: "session-1",
        content: "hello",
        controls: { locale: "en-US" },
        addressedParticipantId: null,
      }),
    );
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

  it("addresses a message through an in-composer participant mention", async () => {
    mockParticipants.push(
      {
        id: "part_host",
        session_id: "session-1",
        principal_id: "principal-host",
        agent_id: "agent-1",
        display_name: "Host",
        kind: "agent",
        role: "host",
        joined_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "part_guest_123456",
        session_id: "session-1",
        principal_id: "principal-guest",
        agent_id: "agent-2",
        display_name: "Researcher",
        kind: "agent",
        role: "member",
        joined_at: "2026-01-01T00:00:00Z",
      },
    );
    mockSessionContext.sendMessage.mutateAsync.mockResolvedValueOnce(undefined);

    render(<ChatPanel />);

    const textarea = screen.getByPlaceholderText("Type a message... (Enter to send)");
    fireEvent.change(textarea, { target: { value: "@", selectionStart: 1 } });

    expect(screen.getByRole("listbox", { name: "Address a participant" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /Researcher/ })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /Host/ })).not.toBeInTheDocument();

    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });
    expect(screen.getByTestId("participant-mention-token")).toHaveTextContent("Researcher");

    fireEvent.change(textarea, { target: { value: "Summarize the findings", selectionStart: 22 } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    await waitFor(() =>
      expect(mockSessionContext.sendMessage.mutateAsync).toHaveBeenCalledWith({
        sessionId: "session-1",
        content: "Summarize the findings",
        controls: { locale: "en-US" },
        addressedParticipantId: "part_guest_123456",
      }),
    );
    await waitFor(() =>
      expect(screen.queryByTestId("participant-mention-token")).not.toBeInTheDocument(),
    );
  });

  it("disambiguates duplicate participant display names and supports mention removal", () => {
    mockParticipants.push(
      {
        id: "part_guest_aaaaaa",
        session_id: "session-1",
        principal_id: "principal-a",
        display_name: "Helper",
        kind: "agent",
        role: "member",
        joined_at: "2026-01-01T00:00:00Z",
      },
      {
        id: "part_guest_bbbbbb",
        session_id: "session-1",
        principal_id: "principal-b",
        display_name: "Helper",
        kind: "agent",
        role: "member",
        joined_at: "2026-01-01T00:00:00Z",
      },
    );

    render(<ChatPanel />);

    const textarea = screen.getByPlaceholderText("Type a message... (Enter to send)");
    fireEvent.change(textarea, { target: { value: "@help", selectionStart: 5 } });
    fireEvent.click(screen.getByRole("option", { name: /Helper \(bbbbbb\)/ }));

    expect(screen.getByTestId("participant-mention-token")).toHaveTextContent("Helper (bbbbbb)");
    fireEvent.keyDown(textarea, { key: "Backspace", code: "Backspace" });
    expect(screen.queryByTestId("participant-mention-token")).not.toBeInTheDocument();
  });

  it("shows mention empty states without interfering with slash commands", () => {
    mockUseSessionCommands.mockReturnValue({
      data: {
        commands: [{ name: "clear", description: "Clear the session", source: "system" }],
      },
    });

    render(<ChatPanel />);

    const textarea = screen.getByPlaceholderText(
      "Type a message or / for commands... (Enter to send)",
    );
    fireEvent.change(textarea, { target: { value: "@", selectionStart: 1 } });
    expect(screen.getByText("No participants available")).toBeInTheDocument();

    fireEvent.keyDown(textarea, { key: "Escape", code: "Escape" });
    expect(screen.queryByText("No participants available")).not.toBeInTheDocument();

    fireEvent.change(textarea, { target: { value: "/", selectionStart: 1 } });
    expect(
      screen.queryByRole("listbox", { name: "Address a participant" }),
    ).not.toBeInTheDocument();
  });

  it("opens the command menu when discovery finishes after slash is typed", async () => {
    let commandData: { data?: { commands: Array<Record<string, unknown>> } } = {};
    mockUseSessionCommands.mockImplementation(() => commandData);
    const { rerender } = render(<ChatPanel />);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "/" } });

    commandData = {
      data: {
        commands: [
          {
            name: "btw",
            description: "Ask a side question",
            source: "system",
          },
        ],
      },
    };
    rerender(<ChatPanel />);

    expect(await screen.findByRole("button", { name: "/btw" })).toBeInTheDocument();
  });

  it("does not render a top border between messages and composer", () => {
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });

    render(<ChatPanel />);

    const composer = screen.getByPlaceholderText("Type a message... (Enter to send)");
    const composerShell = composer.closest("form")?.parentElement;

    expect(composerShell).not.toBeNull();
    expect(composerShell).not.toHaveClass("border-t");
  });

  it("drives one combined model/effort menu with reasoning support from the active model", () => {
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });
    mockSessionContext.llmModel = {
      id: "model-1",
      provider_id: "provider-1",
      model_id: "gpt-5.4",
      display_name: "GPT-5.4",
      capabilities: ["chat"],
      enabled: true,
      healthy: true,
      created_at: "2025-01-01T00:00:00Z",
      updated_at: "2025-01-01T00:00:00Z",
      provider_name: "OpenAI",
      provider_type: "openai",
      is_favorite: false,
      profile: {
        name: "GPT-5.4",
        family: "gpt-5",
        attachment: false,
        reasoning: true,
        temperature: true,
        tool_call: true,
        structured_output: true,
        open_weights: false,
        reasoning_effort: {
          default: "medium",
          values: [{ value: "medium", name: "Medium" }],
        },
      },
    };

    render(<ChatPanel />);

    expect(mockModelEffortMenu).toHaveBeenCalled();
    const props = mockModelEffortMenu.mock.calls.at(-1)?.[0];
    expect(props?.supportsReasoning).toBe(true);
    expect(props?.modelTriggerLabel).toBe("Default · GPT-5.4");
    expect(Array.isArray(props?.recentModels)).toBe(true);
  });

  it("executes /btw without sending a chat message and shows the overlay answer", async () => {
    mockUseSessionCommands.mockReturnValue({
      data: {
        commands: [
          {
            name: "btw",
            description: "Ask a side question",
            source: "system",
            args: [{ name: "question", description: "The side question", required: true }],
          },
        ],
      },
    });
    mockExecuteSessionCommand.mockResolvedValue({
      success: true,
      message: "Side answer",
    });

    render(<ChatPanel />);

    const textarea = screen.getByPlaceholderText(
      "Type a message or / for commands... (Enter to send)",
    );
    fireEvent.change(textarea, { target: { value: "/btw why is this running" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    await waitFor(() =>
      expect(mockExecuteSessionCommand).toHaveBeenCalledWith("session-1", {
        name: "btw",
        arguments: "why is this running",
        controls: { locale: "en-US" },
      }),
    );

    expect(mockSessionContext.sendMessage.mutateAsync).not.toHaveBeenCalled();
    expect(await screen.findByText("Side answer")).toBeInTheDocument();
    expect(screen.getByText("/btw")).toBeInTheDocument();
  });

  it("contains command execution errors without breaking later messages", async () => {
    mockUseSessionCommands.mockReturnValue({
      data: {
        commands: [
          {
            name: "btw",
            description: "Ask a side question",
            source: "system",
            args: [{ name: "question", description: "The side question", required: true }],
          },
        ],
      },
    });
    mockExecuteSessionCommand.mockRejectedValue(new Error("private provider diagnostic"));
    mockSessionContext.sendMessage.mutateAsync.mockResolvedValue({ id: "message-1" });

    render(<ChatPanel />);
    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "/btw question" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    expect(await screen.findByText("Command execution failed. Try again.")).toBeInTheDocument();
    expect(screen.queryByText("private provider diagnostic")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));

    fireEvent.change(textarea, { target: { value: "ordinary message" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });
    await waitFor(() => expect(mockSessionContext.sendMessage.mutateAsync).toHaveBeenCalled());
  });

  it("fills a required system command from the menu and waits for arguments", async () => {
    mockUseSessionCommands.mockReturnValue({
      data: {
        commands: [
          {
            name: "btw",
            description: "Ask a side question",
            source: "system",
            args: [{ name: "question", description: "The side question", required: true }],
          },
        ],
      },
    });

    render(<ChatPanel />);
    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "/" } });
    fireEvent.click(await screen.findByRole("button", { name: "/btw" }));

    expect(textarea).toHaveValue("/btw ");
    expect(textarea).toHaveFocus();
    expect(mockExecuteSessionCommand).not.toHaveBeenCalled();

    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });
    expect(textarea).toHaveValue("/btw ");
    expect(mockExecuteSessionCommand).not.toHaveBeenCalled();
  });

  it("fills a skill command and sends it through ordinary message semantics", async () => {
    mockUseSessionCommands.mockReturnValue({
      data: {
        commands: [
          {
            name: "review",
            description: "Review the current work",
            source: "skill",
          },
        ],
      },
    });
    mockSessionContext.sendMessage.mutateAsync.mockResolvedValue({ id: "message-1" });

    render(<ChatPanel />);
    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "/" } });
    fireEvent.click(await screen.findByRole("button", { name: "/review" }));
    expect(textarea).toHaveValue("/review ");

    fireEvent.change(textarea, { target: { value: "/review inspect this" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });
    await waitFor(() =>
      expect(mockSessionContext.sendMessage.mutateAsync).toHaveBeenCalledWith({
        sessionId: "session-1",
        content: "/review inspect this",
        controls: { locale: "en-US" },
        addressedParticipantId: null,
      }),
    );
    expect(mockExecuteSessionCommand).not.toHaveBeenCalled();
  });

  it("keeps ordinary messages working when command discovery fails", async () => {
    mockUseSessionCommands.mockReturnValue({
      data: undefined,
      error: new Error("command discovery unavailable"),
    });
    mockSessionContext.sendMessage.mutateAsync.mockResolvedValue({ id: "message-1" });

    render(<ChatPanel />);
    const textarea = screen.getByPlaceholderText("Type a message... (Enter to send)");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    await waitFor(() => expect(mockSessionContext.sendMessage.mutateAsync).toHaveBeenCalled());
  });

  it("clears composer state when the resolved session is replaced", async () => {
    mockUseSessionCommands.mockReturnValue({ data: { commands: [] } });
    const { rerender } = render(<ChatPanel />);
    const textarea = screen.getByRole("combobox");
    fireEvent.change(textarea, { target: { value: "stale draft" } });

    mockSessionContext.sessionId = "session-2";
    rerender(<ChatPanel />);

    await waitFor(() => expect(screen.getByRole("combobox")).toHaveValue(""));
    expect(mockUseSessionCommands).toHaveBeenLastCalledWith("session-2");
  });

  it("renders a chat error alert for failed turns", () => {
    mockSessionContext.chatEvents = [
      {
        id: "evt-failed-1",
        type: "turn.failed",
        session_id: "session-1",
        ts: new Date().toISOString(),
        context: { turn_id: "turn-1" },
        data: {
          turn_id: "turn-1",
          error: "backend temporarily unavailable",
          error_code: "dependency_unavailable",
        },
      },
    ];

    render(<ChatPanel />);

    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(
      screen.getByText("Execution stopped because a required dependency is unavailable."),
    ).toBeInTheDocument();
    expect(screen.queryByText("backend temporarily unavailable")).not.toBeInTheDocument();
  });

  it("renders one alert when an error message and turn failure describe the same turn", () => {
    const context = { turn_id: "turn-1", input_message_id: "message-1" };
    mockSessionContext.chatEvents = [
      {
        id: "evt-error-message-1",
        type: "output.message.completed",
        session_id: "session-1",
        ts: new Date().toISOString(),
        context,
        data: {
          message: {
            id: "message-2",
            role: "agent",
            content: [{ type: "text", text: "Provider quota exhausted" }],
          },
          error_code: "provider_quota_exhausted",
        },
      },
      {
        id: "evt-failed-1",
        type: "turn.failed",
        session_id: "session-1",
        ts: new Date().toISOString(),
        context,
        data: {
          turn_id: "turn-1",
          error: "Provider quota exhausted",
          error_code: "provider_quota_exhausted",
        },
      },
    ];
    mockSessionContext.getMessageText.mockReturnValue(
      "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue.",
    );

    render(<ChatPanel />);

    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(
      screen.getByText("The AI provider account is out of credits or quota.", { exact: false }),
    ).toBeInTheDocument();
  });

  it("hides raw failed turn diagnostics when no structured error code is available", () => {
    mockSessionContext.chatEvents = [
      {
        id: "evt-failed-raw-1",
        type: "turn.failed",
        session_id: "session-1",
        ts: new Date().toISOString(),
        context: { turn_id: "turn-1" },
        data: {
          turn_id: "turn-1",
          error: "provider token leaked in diagnostic payload",
        },
      },
    ];

    render(<ChatPanel />);

    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("Message failed.")).toBeInTheDocument();
    expect(
      screen.queryByText("provider token leaked in diagnostic payload"),
    ).not.toBeInTheDocument();
  });

  it("renders a chat error alert when sending a message fails", async () => {
    mockSessionContext.sendMessage.mutateAsync.mockRejectedValueOnce(
      new Error("backend temporarily unavailable"),
    );

    render(<ChatPanel />);

    const textarea = screen.getByPlaceholderText("Type a message... (Enter to send)");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", code: "Enter" });

    expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("backend temporarily unavailable")).toBeInTheDocument();
  });

  it("shows an actionable microphone permission error without starting voice", async () => {
    const getUserMedia = jest
      .fn()
      .mockRejectedValue(new DOMException("Permission denied", "NotAllowedError"));
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia },
    });
    Object.defineProperty(globalThis, "RTCPeerConnection", {
      configurable: true,
      value: class MockRTCPeerConnection {},
    });
    mockUseFeatureFlag.mockReturnValue(true);

    render(<ChatPanel />);

    fireEvent.click(screen.getByTitle("Start voice session"));

    expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
    expect(
      screen.getByText("Check your browser microphone permissions, then try starting voice again."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Microphone access is blocked. Allow microphone access in your browser settings and try again.",
      ),
    ).toBeInTheDocument();
    expect(mockStartSessionVoice).not.toHaveBeenCalled();
  });

  it("shows a provider setup voice error for backend failures", async () => {
    const getUserMedia = jest.fn().mockResolvedValue({
      getTracks: () => [{ stop: jest.fn() }],
    });
    class MockRTCPeerConnection {
      ontrack: ((event: { streams: MediaStream[] }) => void) | null = null;
      addTrack = jest.fn();
      close = jest.fn();
      createOffer = jest.fn().mockResolvedValue({ type: "offer", sdp: "local-sdp" });
      setLocalDescription = jest.fn();
      setRemoteDescription = jest.fn();
    }
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia },
    });
    Object.defineProperty(globalThis, "RTCPeerConnection", {
      configurable: true,
      value: MockRTCPeerConnection,
    });
    mockUseFeatureFlag.mockReturnValue(true);
    mockStartSessionVoice.mockRejectedValue(new ApiError(502, "Bad Gateway", "Bad gateway"));

    render(<ChatPanel />);

    fireEvent.click(screen.getByTitle("Start voice session"));

    expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("Voice service is unavailable.")).toBeInTheDocument();
    expect(
      screen.getByText(
        "The voice service could not start a realtime call. Check provider configuration, then try again.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(
        "Check your browser microphone permissions, then try starting voice again.",
      ),
    ).not.toBeInTheDocument();
  });
});
