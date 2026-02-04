import { render, screen } from "@testing-library/react";
import { ToolCallCard } from "@/components/chat/tool-call-card";
import type { Message } from "@/lib/api/types";

// Helper to create tool call message (as agent message with tool_call in content)
function createToolCallMessage(overrides?: Partial<Message>): Message {
  return {
    id: "msg-tool-call-1",
    session_id: "session-1",
    sequence: 1,
    role: "agent",
    content: [
      {
        type: "tool_call" as const,
        id: "call_123",
        name: "get_current_time",
        arguments: { timezone: "UTC" },
      },
    ],
    tool_call_id: null,
    created_at: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

// Helper to create tool result message
function createToolResultMessage(overrides?: Partial<Message>): Message {
  return {
    id: "msg-tool-result-1",
    session_id: "session-1",
    sequence: 2,
    role: "tool_result",
    content: [
      {
        type: "tool_result" as const,
        tool_call_id: "call_123",
        result: "2025-01-01T12:00:00Z",
      },
    ],
    tool_call_id: "call_123",
    created_at: "2025-01-01T00:00:01Z",
    ...overrides,
  };
}

describe("ToolCallCard", () => {
  describe("rendering", () => {
    it("renders tool call name with colon", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("get_current_time:")).toBeInTheDocument();
    });

    it("renders arguments inline", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("timezone: UTC")).toBeInTheDocument();
    });

    it("does not render arguments when empty", () => {
      const toolCall = createToolCallMessage({
        content: [
          {
            type: "tool_call" as const,
            id: "call_123",
            name: "noop",
            arguments: {},
          },
        ],
      });
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("noop:")).toBeInTheDocument();
    });
  });

  describe("status display", () => {
    it("shows executing indicator when no tool result provided", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("> ... executing ...")).toBeInTheDocument();
    });

    it("shows result when tool result is successful", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage();
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText(/2025-01-01T12:00:00Z/)).toBeInTheDocument();
      expect(screen.queryByText("> ... executing ...")).not.toBeInTheDocument();
    });

    it("shows error message when tool result has error", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: [
          {
            type: "tool_result" as const,
            tool_call_id: "call_123",
            error: "Something went wrong",
          },
        ],
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText("> Error: Something went wrong")).toBeInTheDocument();
    });
  });

  describe("result display", () => {
    it("displays result preview", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: [
          {
            type: "tool_result" as const,
            tool_call_id: "call_123",
            result: "2025-01-01T12:00:00Z",
          },
        ],
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText(/2025-01-01T12:00:00Z/)).toBeInTheDocument();
    });

    it("does not display result section when incomplete", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.queryByText(/Result:/)).not.toBeInTheDocument();
    });
  });

  describe("different tool types", () => {
    it("renders tool with complex arguments", () => {
      const toolCall = createToolCallMessage({
        content: [
          {
            type: "tool_call" as const,
            id: "call_456",
            name: "http_get",
            arguments: {
              url: "https://api.example.com/data",
              headers: {
                "Content-Type": "application/json",
              },
            },
          },
        ],
      });
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("http_get:")).toBeInTheDocument();
    });

    it("renders tool with no result value", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: [
          {
            type: "tool_result" as const,
            tool_call_id: "call_123",
            result: undefined,
          },
        ],
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      // Should not show executing indicator when complete
      expect(screen.queryByText("> ... executing ...")).not.toBeInTheDocument();
    });
  });
});
