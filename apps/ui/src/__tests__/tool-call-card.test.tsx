import { render, screen, fireEvent } from "@testing-library/react";
import { ToolCallCard } from "@/components/chat/tool-call-card";
import type { Message } from "@/lib/api/types";

// Helper to create tool call message
function createToolCallMessage(overrides?: Partial<Message>): Message {
  return {
    id: "msg-tool-call-1",
    session_id: "session-1",
    sequence: 1,
    role: "tool_call",
    content: {
      id: "call_123",
      name: "get_current_time",
      arguments: { timezone: "UTC" },
    },
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
    content: {
      result: "2025-01-01T12:00:00Z",
    },
    tool_call_id: "call_123",
    created_at: "2025-01-01T00:00:01Z",
    ...overrides,
  };
}

describe("ToolCallCard", () => {
  describe("rendering", () => {
    it("renders tool call name", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("get_current_time")).toBeInTheDocument();
    });

    it("renders 'Step' label", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("Step")).toBeInTheDocument();
    });

    it("renders Arguments button when arguments exist", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByRole("button", { name: /arguments/i })).toBeInTheDocument();
    });

    it("does not render Arguments button when arguments are empty", () => {
      const toolCall = createToolCallMessage({
        content: {
          id: "call_123",
          name: "noop",
          arguments: {},
        },
      });
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.queryByRole("button", { name: /arguments/i })).not.toBeInTheDocument();
    });
  });

  describe("status display", () => {
    it("shows 'Running...' badge when no tool result provided", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("Running...")).toBeInTheDocument();
    });

    it("shows 'Done' badge when tool result is successful", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage();
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText("Done")).toBeInTheDocument();
      expect(screen.queryByText("Running...")).not.toBeInTheDocument();
    });

    it("shows 'Failed' badge when tool result has error", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: {
          error: "Something went wrong",
        },
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText("Failed")).toBeInTheDocument();
      expect(screen.queryByText("Done")).not.toBeInTheDocument();
    });
  });

  describe("arguments expansion", () => {
    it("arguments are collapsed by default", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      // Arguments JSON should not be visible initially
      expect(screen.queryByText(/"timezone"/)).not.toBeInTheDocument();
    });

    it("expands arguments when button is clicked", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      const argumentsButton = screen.getByRole("button", { name: /arguments/i });
      fireEvent.click(argumentsButton);

      // Now arguments should be visible
      expect(screen.getByText(/"timezone": "UTC"/)).toBeInTheDocument();
    });

    it("collapses arguments when button is clicked again", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      const argumentsButton = screen.getByRole("button", { name: /arguments/i });

      // Expand
      fireEvent.click(argumentsButton);
      expect(screen.getByText(/"timezone": "UTC"/)).toBeInTheDocument();

      // Collapse
      fireEvent.click(argumentsButton);
      expect(screen.queryByText(/"timezone": "UTC"/)).not.toBeInTheDocument();
    });
  });

  describe("result display", () => {
    it("displays successful result", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: {
          result: "2025-01-01T12:00:00Z",
        },
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText("Result:")).toBeInTheDocument();
      expect(screen.getByText("2025-01-01T12:00:00Z")).toBeInTheDocument();
    });

    it("displays JSON result for objects", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: {
          result: { time: "12:00", date: "2025-01-01" },
        },
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText("Result:")).toBeInTheDocument();
      expect(screen.getByText(/"time": "12:00"/)).toBeInTheDocument();
    });

    it("displays error message when tool fails", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: {
          error: "Network timeout occurred",
        },
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      expect(screen.getByText("Network timeout occurred")).toBeInTheDocument();
      expect(screen.queryByText("Result:")).not.toBeInTheDocument();
    });

    it("does not display result section when incomplete", () => {
      const toolCall = createToolCallMessage();
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.queryByText("Result:")).not.toBeInTheDocument();
    });
  });

  describe("different tool types", () => {
    it("renders tool with complex arguments", () => {
      const toolCall = createToolCallMessage({
        content: {
          id: "call_456",
          name: "http_get",
          arguments: {
            url: "https://api.example.com/data",
            headers: {
              "Content-Type": "application/json",
              "Authorization": "Bearer token123",
            },
            timeout: 5000,
          },
        },
      });
      render(<ToolCallCard toolCall={toolCall} />);

      expect(screen.getByText("http_get")).toBeInTheDocument();

      // Expand to see arguments
      fireEvent.click(screen.getByRole("button", { name: /arguments/i }));
      expect(screen.getByText(/"url": "https:\/\/api.example.com\/data"/)).toBeInTheDocument();
    });

    it("renders tool with no result value (null/undefined)", () => {
      const toolCall = createToolCallMessage();
      const toolResult = createToolResultMessage({
        content: {
          result: undefined,
        },
      });
      render(<ToolCallCard toolCall={toolCall} toolResult={toolResult} />);

      // Should show Done but no result section
      expect(screen.getByText("Done")).toBeInTheDocument();
      expect(screen.queryByText("Result:")).not.toBeInTheDocument();
    });
  });
});
