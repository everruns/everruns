"use client";

import { useState } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Textarea } from "@/components/ui/textarea";
import { Bot, Send, ArrowLeft, ImagePlus } from "lucide-react";
import { ToolCallCardFromEvent } from "@/components/chat/tool-call-card-from-event";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ThinkingIndicator } from "@/components/thinking-indicator";
import type { Event, ToolCallCompletedData } from "@/lib/api/types";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

// Sample events that match the real chat UI data structure
const sampleChatEvents: Event[] = [
  {
    id: "evt-1",
    type: "message.user",
    ts: new Date(Date.now() - 300000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-1" },
    data: {
      message: {
        id: "msg-1",
        session_id: "session-demo",
        sequence: 1,
        role: "user" as const,
        content: [{ type: "text" as const, text: "Can you help me analyze the project structure and run the tests?" }],
        tool_call_id: null,
        created_at: new Date(Date.now() - 300000).toISOString(),
      },
    },
  },
  {
    id: "evt-2",
    type: "message.agent",
    ts: new Date(Date.now() - 290000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-1" },
    data: {
      message: {
        id: "msg-2",
        session_id: "session-demo",
        sequence: 2,
        role: "agent" as const,
        content: [
          { type: "text" as const, text: "I'll analyze the project structure and run the tests for you." },
          {
            type: "tool_call" as const,
            id: "tc-1",
            name: "list_files",
            arguments: { path: "/home/user/project", recursive: true },
          },
          {
            type: "tool_call" as const,
            id: "tc-2",
            name: "bash",
            arguments: { command: "cargo test --workspace" },
          },
        ],
        tool_call_id: null,
        created_at: new Date(Date.now() - 290000).toISOString(),
      },
      metadata: {
        model: "claude-sonnet-4-20250514",
      },
      usage: {
        input_tokens: 256,
        output_tokens: 128,
      },
    },
  },
  {
    id: "evt-3",
    type: "tool.call_completed",
    ts: new Date(Date.now() - 280000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-1" },
    data: {
      tool_call_id: "tc-1",
      name: "list_files",
      arguments: { path: "/home/user/project", recursive: true },
      result: [{ type: "text" as const, text: "src/\nsrc/main.rs\nsrc/lib.rs\nCargo.toml\nREADME.md\ntests/\ntests/integration.rs" }],
    } as ToolCallCompletedData,
  },
  {
    id: "evt-4",
    type: "tool.call_completed",
    ts: new Date(Date.now() - 270000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-1" },
    data: {
      tool_call_id: "tc-2",
      name: "bash",
      arguments: { command: "cargo test --workspace" },
      result: [{ type: "text" as const, text: "running 24 tests\ntest storage::tests::test_create ... ok\ntest storage::tests::test_list ... ok\ntest api::tests::test_health ... ok\ntest api::tests::test_session ... ok\n...\ntest result: ok. 24 passed; 0 failed; 0 ignored" }],
    } as ToolCallCompletedData,
  },
  {
    id: "evt-5",
    type: "message.agent",
    ts: new Date(Date.now() - 260000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-1" },
    data: {
      message: {
        id: "msg-3",
        session_id: "session-demo",
        sequence: 5,
        role: "agent" as const,
        content: [{ type: "text" as const, text: "Great news! Your project has a clean structure with 7 files and all 24 tests passed successfully.\n\nThe project includes:\n- Main source files in src/\n- Integration tests in tests/\n- Standard Cargo.toml and README" }],
        tool_call_id: null,
        created_at: new Date(Date.now() - 260000).toISOString(),
      },
      metadata: {
        model: "claude-sonnet-4-20250514",
      },
      usage: {
        input_tokens: 512,
        output_tokens: 96,
      },
    },
  },
  {
    id: "evt-6",
    type: "message.user",
    ts: new Date(Date.now() - 200000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-2" },
    data: {
      message: {
        id: "msg-4",
        session_id: "session-demo",
        sequence: 6,
        role: "user" as const,
        content: [{ type: "text" as const, text: "Can you show me the main.rs file?" }],
        tool_call_id: null,
        created_at: new Date(Date.now() - 200000).toISOString(),
      },
    },
  },
  {
    id: "evt-7",
    type: "message.agent",
    ts: new Date(Date.now() - 190000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-2" },
    data: {
      message: {
        id: "msg-5",
        session_id: "session-demo",
        sequence: 7,
        role: "agent" as const,
        content: [
          { type: "text" as const, text: "I'll read the main.rs file for you." },
          {
            type: "tool_call" as const,
            id: "tc-3",
            name: "read_file",
            arguments: { path: "/home/user/project/src/main.rs" },
          },
        ],
        tool_call_id: null,
        created_at: new Date(Date.now() - 190000).toISOString(),
      },
      metadata: {
        model: "claude-sonnet-4-20250514",
      },
      usage: {
        input_tokens: 768,
        output_tokens: 64,
      },
    },
  },
  {
    id: "evt-8",
    type: "tool.call_completed",
    ts: new Date(Date.now() - 180000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-2" },
    data: {
      tool_call_id: "tc-3",
      name: "read_file",
      arguments: { path: "/home/user/project/src/main.rs" },
      result: [{ type: "text" as const, text: "use everruns_core::Agent;\n\nfn main() {\n    println!(\"Starting Everruns...\");\n    let agent = Agent::new(\"demo\");\n    agent.run();\n}" }],
    } as ToolCallCompletedData,
  },
  {
    id: "evt-9",
    type: "message.agent",
    ts: new Date(Date.now() - 170000).toISOString(),
    session_id: "session-demo",
    context: { turn_id: "turn-2" },
    data: {
      message: {
        id: "msg-6",
        session_id: "session-demo",
        sequence: 9,
        role: "agent" as const,
        content: [{ type: "text" as const, text: "Here's the main.rs file. It's a simple entry point that creates and runs an Agent from the everruns_core crate." }],
        tool_call_id: null,
        created_at: new Date(Date.now() - 170000).toISOString(),
      },
      metadata: {
        model: "claude-sonnet-4-20250514",
      },
      usage: {
        input_tokens: 1024,
        output_tokens: 48,
      },
    },
  },
];

// Build tool results map
const toolResultsMap = new Map<string, ToolCallCompletedData>();
sampleChatEvents.forEach((event) => {
  if (event.type === "tool.call_completed") {
    const data = event.data as ToolCallCompletedData;
    toolResultsMap.set(data.tool_call_id, data);
  }
});

// Helper to get text content from message
function getMessageText(data: { message?: { content?: Array<{ type: string; text?: string }> } }): string {
  if (!data.message?.content) return "";
  return data.message.content
    .filter((part): part is { type: "text"; text: string } => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

// Helper to get tool calls from message
function getToolCalls(data: { message?: { content?: Array<{ type: string; id?: string; name?: string; arguments?: Record<string, unknown> }> } }) {
  if (!data.message?.content) return [];
  return data.message.content.filter(
    (part): part is { type: "tool_call"; id: string; name: string; arguments: Record<string, unknown> } =>
      part.type === "tool_call"
  );
}

export default function ChatDemoPage() {
  const [inputValue, setInputValue] = useState("");
  const [showThinking, setShowThinking] = useState(false);

  // Show 404-like message in production
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="text-muted-foreground mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-screen">
      {/* Header */}
      <div className="border-b p-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <Link
              href="/dev"
              className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
            >
              <ArrowLeft className="w-4 h-4 mr-2" />
              Back
            </Link>
            <div>
              <h1 className="text-lg font-semibold">Chat UI Demo</h1>
              <p className="text-sm text-muted-foreground">
                Showing Minimal + Icon style with sample conversation
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Badge variant="outline">claude-sonnet-4</Badge>
            <Badge variant="secondary">Demo Mode</Badge>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setShowThinking(!showThinking)}
            >
              {showThinking ? "Hide" : "Show"} Thinking
            </Button>
          </div>
        </div>
      </div>

      {/* Messages area */}
      <div className="flex-1 overflow-auto p-4 space-y-4">
        {sampleChatEvents.map((event) => {
          // Skip tool.call_completed - rendered inline with agent messages
          if (event.type === "tool.call_completed") {
            return null;
          }

          const isUser = event.type === "message.user";
          const data = event.data as { message?: { content?: Array<{ type: string; text?: string }> } };
          const textContent = getMessageText(data);
          const toolCalls = isUser ? [] : getToolCalls(data as Parameters<typeof getToolCalls>[0]);

          return (
            <div key={event.id} className="space-y-2">
              {/* Render text content */}
              {textContent && (
                <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                  {isUser ? (
                    /* User message - subtle border, rounded */
                    <div className="max-w-[85%] border border-border/60 rounded-xl px-3 py-2">
                      <div className="flex items-start gap-2">
                        <div className="flex-1">
                          <p className="text-sm whitespace-pre-wrap">{textContent}</p>
                        </div>
                        <MessageInfoIcon event={event} />
                      </div>
                    </div>
                  ) : (
                    /* Agent message - flat with robot icon */
                    <div className="w-full flex items-start gap-2">
                      <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground/60" />
                      <div className="flex-1 flex items-start gap-2">
                        <div className="flex-1">
                          <p className="text-sm whitespace-pre-wrap text-foreground/90">{textContent}</p>
                        </div>
                        <MessageInfoIcon event={event} />
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* Render tool calls from agent message */}
              {toolCalls.length > 0 && (
                <div className="ml-6 space-y-1">
                  {toolCalls.map((tc) => {
                    const toolResult = toolResultsMap.get(tc.id);
                    return (
                      <ToolCallCardFromEvent key={tc.id} toolCall={tc} toolResult={toolResult} />
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}

        {/* Optional thinking indicator */}
        {showThinking && (
          <div className="flex justify-start">
            <div className="w-full flex items-start gap-2">
              <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground/60" />
              <div className="flex-1">
                <ThinkingIndicator model="claude-sonnet-4" />
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Input area */}
      <div className="border-t p-4">
        <form className="flex gap-2" onSubmit={(e) => e.preventDefault()}>
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="h-[60px] w-[60px] flex-shrink-0"
            title="Attach images"
          >
            <ImagePlus className="h-5 w-5" />
          </Button>

          <Textarea
            value={inputValue}
            onChange={(e) => setInputValue(e.target.value)}
            placeholder="Type a message... (demo mode - input disabled)"
            className="min-h-[60px] resize-none"
            disabled
          />

          <Button
            type="submit"
            size="icon"
            className="h-[60px] w-[60px] flex-shrink-0"
            disabled
          >
            <Send className="h-5 w-5" />
          </Button>
        </form>
      </div>
    </div>
  );
}
