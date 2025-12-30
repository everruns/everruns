"use client";

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Bot, User, Wrench, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { ToolCallCard } from "@/components/chat/tool-call-card";
import type { Message } from "@/lib/api/types";
import type { AggregatedToolCall } from "@/hooks/use-sse-events";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

// ============================================
// MessageBubble component (copied from chat-messages for showcase)
// ============================================

function MessageBubble({
  messageRole,
  content,
  isStreaming,
}: {
  messageRole: string;
  content: string;
  isStreaming?: boolean;
}) {
  const isUser = messageRole === "user";

  return (
    <div className={cn("flex gap-3", isUser && "flex-row-reverse")}>
      <Avatar className="h-8 w-8 shrink-0">
        <AvatarFallback className={cn(isUser ? "bg-primary" : "bg-muted")}>
          {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
        </AvatarFallback>
      </Avatar>
      <div
        className={cn(
          "rounded-lg px-4 py-2 max-w-[80%]",
          isUser ? "bg-primary text-primary-foreground" : "bg-muted"
        )}
      >
        <p className="whitespace-pre-wrap">{content}</p>
        {isStreaming && (
          <span className="inline-block w-2 h-4 bg-current opacity-75 animate-pulse ml-0.5" />
        )}
      </div>
    </div>
  );
}

// ============================================
// ToolCallBubble component (copied from chat-messages for showcase)
// ============================================

function ToolCallBubble({ toolCall }: { toolCall: AggregatedToolCall }) {
  return (
    <div className="flex gap-3">
      <Avatar className="h-8 w-8 shrink-0">
        <AvatarFallback className="bg-purple-100">
          <Wrench className="h-4 w-4 text-purple-600" />
        </AvatarFallback>
      </Avatar>
      <div className="border rounded-lg p-3 max-w-[80%] bg-purple-50">
        <div className="flex items-center gap-2 mb-2">
          <span className="font-medium text-sm">{toolCall.name}</span>
          {toolCall.isComplete ? (
            toolCall.error ? (
              <Badge variant="destructive" className="text-xs">
                Failed
              </Badge>
            ) : (
              <Badge variant="outline" className="bg-green-100 text-green-800 text-xs">
                Done
              </Badge>
            )
          ) : (
            <Badge variant="outline" className="text-xs">
              <Loader2 className="h-3 w-3 mr-1 animate-spin" />
              Running
            </Badge>
          )}
        </div>
        <pre className="text-xs bg-white p-2 rounded overflow-x-auto">
          {JSON.stringify(toolCall.arguments, null, 2)}
        </pre>
        {toolCall.isComplete && toolCall.result !== undefined && (
          <>
            <Separator className="my-2" />
            <pre className="text-xs bg-white p-2 rounded overflow-x-auto max-h-32">
              {JSON.stringify(toolCall.result, null, 2)}
            </pre>
          </>
        )}
        {toolCall.error && (
          <>
            <Separator className="my-2" />
            <p className="text-sm text-destructive">{toolCall.error}</p>
          </>
        )}
      </div>
    </div>
  );
}

// ============================================
// Loading indicator (copied from chat-messages for showcase)
// ============================================

function LoadingIndicator() {
  return (
    <div className="flex gap-3">
      <Avatar className="h-8 w-8 shrink-0">
        <AvatarFallback className="bg-muted">
          <Bot className="h-4 w-4" />
        </AvatarFallback>
      </Avatar>
      <div className="bg-muted rounded-lg px-4 py-2">
        <div className="flex gap-1">
          <div className="w-2 h-2 bg-current rounded-full animate-bounce" />
          <div className="w-2 h-2 bg-current rounded-full animate-bounce delay-75" />
          <div className="w-2 h-2 bg-current rounded-full animate-bounce delay-150" />
        </div>
      </div>
    </div>
  );
}

// ============================================
// Showcase Section Component
// ============================================

function ShowcaseSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="space-y-4">{children}</CardContent>
    </Card>
  );
}

function ShowcaseItem({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <div className="text-sm font-medium text-muted-foreground">{label}</div>
      <div className="border rounded-lg p-4 bg-background">{children}</div>
    </div>
  );
}

// ============================================
// Sample Data
// ============================================

const sampleToolCalls = {
  running: {
    id: "tc-1",
    name: "read_file",
    arguments: { path: "/data/transactions.csv" },
    isComplete: false,
  },
  success: {
    id: "tc-2",
    name: "bash",
    arguments: { command: "python analyze.py --input data.csv" },
    isComplete: true,
    result: "Analysis complete. Found 15,432 transactions totaling $1,965,000 in revenue.",
  },
  successLongResult: {
    id: "tc-3",
    name: "grep",
    arguments: { pattern: "error", path: "/var/log/app.log" },
    isComplete: true,
    result: [
      "/var/log/app.log:142: [ERROR] Connection timeout",
      "/var/log/app.log:298: [ERROR] Database query failed",
      "/var/log/app.log:456: [ERROR] Authentication failed for user admin",
      "/var/log/app.log:789: [ERROR] File not found: config.json",
      "/var/log/app.log:1024: [ERROR] Memory allocation failed",
    ].join("\n"),
  },
  failed: {
    id: "tc-4",
    name: "write_file",
    arguments: { path: "/etc/config.json", content: "{}" },
    isComplete: true,
    error: "Permission denied: Cannot write to /etc/config.json",
  },
} satisfies Record<string, AggregatedToolCall>;

// Sample Message objects for ToolCallCard component
const sampleToolCallMessages = {
  toolCall: {
    id: "tcm-1",
    session_id: "session-1",
    sequence: 5,
    role: "assistant" as const,
    content: [{
      type: "tool_call" as const,
      id: "tc-call-1",
      name: "list_files",
      arguments: { path: "/home/user/project", recursive: true },
    }],
    tool_call_id: null,
    created_at: new Date().toISOString(),
  },
  toolResult: {
    id: "trm-1",
    session_id: "session-1",
    sequence: 6,
    role: "tool_result" as const,
    content: [{
      type: "tool_result" as const,
      tool_call_id: "tc-call-1",
      result: ["src/", "src/main.rs", "src/lib.rs", "Cargo.toml", "README.md"],
    }],
    tool_call_id: "tc-call-1",
    created_at: new Date().toISOString(),
  },
  toolCallExecuting: {
    id: "tcm-2",
    session_id: "session-1",
    sequence: 7,
    role: "assistant" as const,
    content: [{
      type: "tool_call" as const,
      id: "tc-call-2",
      name: "run_tests",
      arguments: { test_filter: "integration", verbose: true },
    }],
    tool_call_id: null,
    created_at: new Date().toISOString(),
  },
  toolCallError: {
    id: "tcm-3",
    session_id: "session-1",
    sequence: 8,
    role: "assistant" as const,
    content: [{
      type: "tool_call" as const,
      id: "tc-call-3",
      name: "delete_file",
      arguments: { path: "/protected/important.txt" },
    }],
    tool_call_id: null,
    created_at: new Date().toISOString(),
  },
  toolResultError: {
    id: "trm-2",
    session_id: "session-1",
    sequence: 9,
    role: "tool_result" as const,
    content: [{
      type: "tool_result" as const,
      tool_call_id: "tc-call-3",
      error: "Access denied: File is protected and cannot be deleted",
    }],
    tool_call_id: "tc-call-3",
    created_at: new Date().toISOString(),
  },
} satisfies Record<string, Message>;

// ============================================
// Main Page Component
// ============================================

export default function DevComponentsPage() {
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
    <div className="min-h-screen bg-muted/30">
      <div className="container mx-auto py-8 px-4">
        <div className="mb-8">
          <h1 className="text-3xl font-bold">Component Showcase</h1>
          <p className="text-muted-foreground mt-2">
            Development-only page to preview UI component states
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <ScrollArea className="h-[calc(100vh-12rem)]">
          <div className="space-y-8 pr-4">
            {/* Message Bubbles Section */}
            <ShowcaseSection
              title="Message Bubbles"
              description="Chat message display for user and assistant messages"
            >
              <ShowcaseItem label="User Message">
                <MessageBubble messageRole="user" content="Hello! Can you help me with a task?" />
              </ShowcaseItem>

              <ShowcaseItem label="User Message (Long)">
                <MessageBubble
                  messageRole="user"
                  content="I need to analyze a large dataset containing customer transactions from the past year. The data includes purchase amounts, dates, product categories, and customer demographics. Can you help me identify patterns and create a summary report?"
                />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message">
                <MessageBubble
                  messageRole="assistant"
                  content="Of course! I'd be happy to help you with that. Let me start by reading the data file."
                />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message (Multiline)">
                <MessageBubble
                  messageRole="assistant"
                  content={"Here's what I found:\n\n1. Total transactions: 15,432\n2. Average order value: $127.50\n3. Top category: Electronics (32%)\n4. Peak sales month: December\n\nWould you like me to dive deeper into any of these areas?"}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message (Streaming)">
                <MessageBubble
                  messageRole="assistant"
                  content="I'm analyzing the data now and will provide you with a comprehensive report"
                  isStreaming
                />
              </ShowcaseItem>

              <ShowcaseItem label="Loading Indicator">
                <LoadingIndicator />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Tool Call Bubbles Section (Streaming) */}
            <ShowcaseSection
              title="Tool Call Bubbles (Streaming View)"
              description="Real-time tool execution display during SSE streaming"
            >
              <ShowcaseItem label="Running">
                <ToolCallBubble toolCall={sampleToolCalls.running} />
              </ShowcaseItem>

              <ShowcaseItem label="Completed (Success)">
                <ToolCallBubble toolCall={sampleToolCalls.success} />
              </ShowcaseItem>

              <ShowcaseItem label="Completed (Long Result)">
                <ToolCallBubble toolCall={sampleToolCalls.successLongResult} />
              </ShowcaseItem>

              <ShowcaseItem label="Failed">
                <ToolCallBubble toolCall={sampleToolCalls.failed} />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Tool Call Cards Section (History) */}
            <ShowcaseSection
              title="Tool Call Cards (History View)"
              description="Compact tool call display for message history"
            >
              <ShowcaseItem label="Completed with Result">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.toolCall}
                    toolResult={sampleToolCallMessages.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Executing">
                <div className="pl-[25px]">
                  <ToolCallCard toolCall={sampleToolCallMessages.toolCallExecuting} />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Failed with Error">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.toolCallError}
                    toolResult={sampleToolCallMessages.toolResultError}
                  />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Combined Chat View */}
            <ShowcaseSection
              title="Combined Chat View"
              description="Example conversation with messages and tool calls"
            >
              <ShowcaseItem label="Full Conversation">
                <div className="space-y-4">
                  <MessageBubble
                    messageRole="user"
                    content="Can you list the files in my project directory?"
                  />
                  <MessageBubble
                    messageRole="assistant"
                    content="Sure! Let me check what files are in your project."
                  />
                  <ToolCallBubble toolCall={sampleToolCalls.success} />
                  <MessageBubble
                    messageRole="assistant"
                    content="I found several files in your project. The main source files are in the src/ directory."
                  />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
