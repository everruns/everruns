"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Bot, ArrowLeft, Zap } from "lucide-react";
import { ToolCallCard } from "@/components/chat/tool-call-card";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ImageAttachments, MessageImage } from "@/components/chat/image-attachments";
import { SessionCard } from "@/components/session/session-card";
import type { Message, Event, Session, LlmModelWithProvider, TokenUsage } from "@/lib/api/types";
import type { PendingImage } from "@/lib/api/images";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

// ============================================
// Showcase Section Components
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
// Message Rendering (from Session UI)
// These match the inline rendering in sessions/[sessionId]/page.tsx
// ============================================

function UserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[90%] bg-gray-500 text-white rounded-lg p-3">
        <p className="text-sm whitespace-pre-wrap">{content}</p>
      </div>
    </div>
  );
}

function AssistantMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-start">
      <div className="w-full bg-muted/60 rounded-lg p-3">
        <div className="flex items-start gap-2">
          <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground" />
          <p className="text-sm whitespace-pre-wrap">{content}</p>
        </div>
      </div>
    </div>
  );
}

// ============================================
// Sample Data for ToolCallCard
// ============================================

const sampleToolCallMessages = {
  // List files tool - completed with result
  listFiles: {
    toolCall: {
      id: "msg-tc-1",
      session_id: "session-1",
      sequence: 5,
      role: "agent" as const,
      content: [{
        type: "tool_call" as const,
        id: "tc-1",
        name: "list_files",
        arguments: { path: "/home/user/project", recursive: true },
      }],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-1",
      session_id: "session-1",
      sequence: 6,
      role: "tool_result" as const,
      content: [{
        type: "tool_result" as const,
        tool_call_id: "tc-1",
        result: ["src/", "src/main.rs", "src/lib.rs", "Cargo.toml", "README.md"],
      }],
      tool_call_id: "tc-1",
      created_at: new Date().toISOString(),
    },
  },
  // Bash command - completed with longer result
  bashCommand: {
    toolCall: {
      id: "msg-tc-2",
      session_id: "session-1",
      sequence: 7,
      role: "agent" as const,
      content: [{
        type: "tool_call" as const,
        id: "tc-2",
        name: "bash",
        arguments: { command: "cargo test --workspace" },
      }],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-2",
      session_id: "session-1",
      sequence: 8,
      role: "tool_result" as const,
      content: [{
        type: "tool_result" as const,
        tool_call_id: "tc-2",
        result: "running 24 tests\ntest storage::tests::test_create_agent ... ok\ntest storage::tests::test_list_agents ... ok\ntest api::tests::test_health_endpoint ... ok\ntest api::tests::test_create_session ... ok\n\ntest result: ok. 24 passed; 0 failed; 0 ignored",
      }],
      tool_call_id: "tc-2",
      created_at: new Date().toISOString(),
    },
  },
  // Tool currently executing
  executing: {
    toolCall: {
      id: "msg-tc-3",
      session_id: "session-1",
      sequence: 9,
      role: "agent" as const,
      content: [{
        type: "tool_call" as const,
        id: "tc-3",
        name: "read_file",
        arguments: { path: "/home/user/project/src/main.rs" },
      }],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    // No toolResult - still executing
  },
  // Tool with error
  error: {
    toolCall: {
      id: "msg-tc-4",
      session_id: "session-1",
      sequence: 10,
      role: "agent" as const,
      content: [{
        type: "tool_call" as const,
        id: "tc-4",
        name: "write_file",
        arguments: { path: "/etc/protected/config.json", content: "{}" },
      }],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-4",
      session_id: "session-1",
      sequence: 11,
      role: "tool_result" as const,
      content: [{
        type: "tool_result" as const,
        tool_call_id: "tc-4",
        error: "Permission denied: Cannot write to /etc/protected/config.json",
      }],
      tool_call_id: "tc-4",
      created_at: new Date().toISOString(),
    },
  },
  // write_todos tool - shows TodoListRenderer
  writeTodos: {
    toolCall: {
      id: "msg-tc-5",
      session_id: "session-1",
      sequence: 12,
      role: "agent" as const,
      content: [{
        type: "tool_call" as const,
        id: "tc-5",
        name: "write_todos",
        arguments: {
          todos: [
            { content: "Review code changes", activeForm: "Reviewing code changes", status: "completed" },
            { content: "Run tests", activeForm: "Running tests", status: "in_progress" },
            { content: "Update documentation", activeForm: "Updating documentation", status: "pending" },
            { content: "Create pull request", activeForm: "Creating pull request", status: "pending" },
          ],
        },
      }],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-5",
      session_id: "session-1",
      sequence: 13,
      role: "tool_result" as const,
      content: [{
        type: "tool_result" as const,
        tool_call_id: "tc-5",
        result: {
          success: true,
          total_tasks: 4,
          pending: 2,
          in_progress: 1,
          completed: 1,
          todos: [
            { content: "Review code changes", activeForm: "Reviewing code changes", status: "completed" },
            { content: "Run tests", activeForm: "Running tests", status: "in_progress" },
            { content: "Update documentation", activeForm: "Updating documentation", status: "pending" },
            { content: "Create pull request", activeForm: "Creating pull request", status: "pending" },
          ],
        },
      }],
      tool_call_id: "tc-5",
      created_at: new Date().toISOString(),
    },
  },
} satisfies Record<string, { toolCall: Message; toolResult?: Message }>;

// Sample todo data for TodoListRenderer directly
const sampleTodoData = {
  executing: {
    arguments: {
      todos: [
        { content: "Analyze requirements", activeForm: "Analyzing requirements", status: "completed" },
        { content: "Implement feature", activeForm: "Implementing feature", status: "in_progress" },
        { content: "Write tests", activeForm: "Writing tests", status: "pending" },
      ],
    },
    isExecuting: true,
  },
  completed: {
    arguments: {
      todos: [
        { content: "Set up database", activeForm: "Setting up database", status: "completed" },
        { content: "Create API endpoints", activeForm: "Creating API endpoints", status: "completed" },
        { content: "Add authentication", activeForm: "Adding authentication", status: "completed" },
      ],
    },
    result: {
      success: true,
      total_tasks: 3,
      pending: 0,
      in_progress: 0,
      completed: 3,
      todos: [
        { content: "Set up database", activeForm: "Setting up database", status: "completed" },
        { content: "Create API endpoints", activeForm: "Creating API endpoints", status: "completed" },
        { content: "Add authentication", activeForm: "Adding authentication", status: "completed" },
      ],
    },
    isExecuting: false,
  },
  withWarning: {
    arguments: {
      todos: [
        { content: "Task 1", activeForm: "Working on task 1", status: "in_progress" },
        { content: "Task 2", activeForm: "Working on task 2", status: "in_progress" },
      ],
    },
    result: {
      success: true,
      total_tasks: 2,
      pending: 0,
      in_progress: 2,
      completed: 0,
      todos: [
        { content: "Task 1", activeForm: "Working on task 1", status: "in_progress" },
        { content: "Task 2", activeForm: "Working on task 2", status: "in_progress" },
      ],
      warning: "Multiple tasks are in progress simultaneously",
    },
    isExecuting: false,
  },
  error: {
    arguments: {
      todos: [],
    },
    error: "Invalid todo list format",
    isExecuting: false,
  },
};

// ============================================
// Token Usage Display
// ============================================

// Helper function to format token counts in a compact way
function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(1)}M`;
  }
  if (tokens >= 1000) {
    return `${(tokens / 1000).toFixed(1)}K`;
  }
  return tokens.toString();
}

// Helper function to calculate total tokens
function totalTokens(usage: TokenUsage): number {
  return usage.input_tokens + usage.output_tokens;
}

// Sample image attachment data
const samplePendingImages: PendingImage[] = [
  {
    tempId: "temp-1",
    file: null,
    uploadPromise: null,
    imageId: "img-uploaded-1",
    filename: "screenshot.png",
    previewUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23e2e8f0' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%2394a3b8' font-size='10'%3EPNG%3C/text%3E%3C/svg%3E",
    status: "uploaded",
  },
  {
    tempId: "temp-2",
    file: new File([], "photo.jpg"),
    uploadPromise: Promise.resolve({ id: "", filename: "", content_type: "", size_bytes: 0, created_at: "" }),
    imageId: null,
    filename: "photo.jpg",
    previewUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23fef3c7' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%23d97706' font-size='10'%3EJPG%3C/text%3E%3C/svg%3E",
    status: "uploading",
  },
  {
    tempId: "temp-3",
    file: null,
    uploadPromise: null,
    imageId: null,
    filename: "failed.gif",
    previewUrl: "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23fee2e2' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%23dc2626' font-size='10'%3EGIF%3C/text%3E%3C/svg%3E",
    status: "error",
    error: "Upload failed",
  },
];

// Sample usage data
const sampleUsageData = {
  small: {
    input_tokens: 128,
    output_tokens: 45,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
  } satisfies TokenUsage,
  medium: {
    input_tokens: 15234,
    output_tokens: 8721,
    cache_read_tokens: 5000,
    cache_creation_tokens: 0,
  } satisfies TokenUsage,
  large: {
    input_tokens: 1250000,
    output_tokens: 875000,
    cache_read_tokens: 500000,
    cache_creation_tokens: 125000,
  } satisfies TokenUsage,
};

// Token Usage Card component (matches agent detail page)
function TokenUsageCard({ usage, title }: { usage: TokenUsage; title: string }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="flex items-center gap-2 p-2 rounded-md border bg-muted/50">
          <Zap className="w-4 h-4 text-yellow-500" />
          <div className="flex-1">
            <p className="text-sm font-medium">{formatTokens(totalTokens(usage))} total</p>
            <p className="text-xs text-muted-foreground">
              {formatTokens(usage.input_tokens)} input / {formatTokens(usage.output_tokens)} output
              {usage.cache_read_tokens ? ` / ${formatTokens(usage.cache_read_tokens)} cached` : ""}
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

// Token Usage Badge component (matches session header)
function TokenUsageBadge({ usage }: { usage: TokenUsage }) {
  return (
    <Badge variant="outline" className="gap-1" title="Token usage (input/output)">
      <Zap className="w-3 h-3" />
      {formatTokens(usage.input_tokens)} / {formatTokens(usage.output_tokens)}
    </Badge>
  );
}

// Sample event data for MessageInfoIcon
const sampleEvents = {
  userMessage: {
    id: "evt-user-123e4567-e89b-12d3-a456-426614174000",
    type: "message.user",
    ts: new Date().toISOString(),
    session_id: "session-1",
    context: { turn_id: "turn-1" },
    data: {
      message: {
        id: "msg-user-1",
        session_id: "session-1",
        sequence: 1,
        role: "user" as const,
        content: [{ type: "text" as const, text: "Hello!" }],
        tool_call_id: null,
        created_at: new Date().toISOString(),
      },
    },
  } satisfies Event,
  agentMessage: {
    id: "evt-agent-987fcdeb-51a2-4bc3-8def-012345678901",
    type: "message.agent",
    ts: new Date().toISOString(),
    session_id: "session-1",
    context: { turn_id: "turn-1" },
    data: {
      message: {
        id: "msg-agent-1",
        session_id: "session-1",
        sequence: 2,
        role: "agent" as const,
        content: [{ type: "text" as const, text: "Hi there! How can I help?" }],
        tool_call_id: null,
        created_at: new Date().toISOString(),
      },
      metadata: {
        model: "claude-sonnet-4-20250514",
        model_id: "model-uuid-123",
        provider_id: "provider-anthropic",
      },
      usage: {
        input_tokens: 128,
        output_tokens: 45,
      },
    },
    metadata: {
      reasoning_effort: "medium",
    },
  } satisfies Event,
  agentMessageWithHighReasoning: {
    id: "evt-agent-abc12345-6789-def0-1234-567890abcdef",
    type: "message.agent",
    ts: new Date(Date.now() - 3600000).toISOString(), // 1 hour ago
    session_id: "session-1",
    context: { turn_id: "turn-2" },
    data: {
      message: {
        id: "msg-agent-2",
        session_id: "session-1",
        sequence: 4,
        role: "agent" as const,
        content: [{ type: "text" as const, text: "Let me analyze this complex problem..." }],
        tool_call_id: null,
        created_at: new Date(Date.now() - 3600000).toISOString(),
      },
      metadata: {
        model: "o1-preview",
        model_id: "model-uuid-456",
        provider_id: "provider-openai",
      },
      usage: {
        input_tokens: 2048,
        output_tokens: 1536,
      },
    },
    metadata: {
      reasoning_effort: "high",
    },
  } satisfies Event,
};

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
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-4"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <h1 className="text-3xl font-bold">Session Chat Components</h1>
          <p className="text-muted-foreground mt-2">
            Components used in the Session UI for chat messages and tool interactions
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <ScrollArea className="h-[calc(100vh-12rem)]">
          <div className="space-y-8 pr-4">
            {/* Message Rendering Section */}
            <ShowcaseSection
              title="Message Rendering"
              description="User and assistant message styles from Session UI (sessions/[sessionId]/page.tsx)"
            >
              <ShowcaseItem label="User Message">
                <UserMessage content="Hello! Can you help me analyze this code?" />
              </ShowcaseItem>

              <ShowcaseItem label="User Message (Long)">
                <UserMessage content="I need to refactor the authentication system to support OAuth 2.0 in addition to the existing session-based auth. The new system should maintain backward compatibility while adding support for multiple identity providers like Google, GitHub, and Microsoft." />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message">
                <AssistantMessage content="I'll help you with that. Let me start by examining the current authentication implementation." />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message (Multiline)">
                <AssistantMessage content={"Here's my analysis of the codebase:\n\n1. Current auth uses session cookies\n2. User model has email/password fields\n3. No OAuth support exists yet\n\nI recommend starting with the OAuth provider abstraction."} />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* MessageInfoIcon Section */}
            <ShowcaseSection
              title="MessageInfoIcon Component"
              description="Small info icon showing message metadata on hover (components/chat/message-info-icon.tsx)"
            >
              <ShowcaseItem label="User Message (Light Variant)">
                <div className="flex justify-end">
                  <div className="max-w-[90%] bg-gray-500 text-white rounded-lg p-3">
                    <div className="flex items-start gap-2">
                      <p className="text-sm whitespace-pre-wrap flex-1">Hello! Can you help me?</p>
                      <MessageInfoIcon event={sampleEvents.userMessage} variant="light" />
                    </div>
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Agent Message (Default Variant)">
                <div className="w-full bg-muted/60 rounded-lg p-3">
                  <div className="flex items-start gap-2">
                    <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground" />
                    <p className="text-sm whitespace-pre-wrap flex-1">Hi there! How can I help?</p>
                    <MessageInfoIcon event={sampleEvents.agentMessage} />
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Agent Message with High Reasoning Effort">
                <div className="w-full bg-muted/60 rounded-lg p-3">
                  <div className="flex items-start gap-2">
                    <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground" />
                    <p className="text-sm whitespace-pre-wrap flex-1">Let me analyze this complex problem...</p>
                    <MessageInfoIcon event={sampleEvents.agentMessageWithHighReasoning} />
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Standalone Icons (hover to see tooltip)">
                <div className="flex items-center gap-4">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Default:</span>
                    <MessageInfoIcon event={sampleEvents.agentMessage} />
                  </div>
                  <div className="flex items-center gap-2 bg-gray-500 rounded px-2 py-1">
                    <span className="text-sm text-white">Light:</span>
                    <MessageInfoIcon event={sampleEvents.userMessage} variant="light" />
                  </div>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* ToolCallCard Section */}
            <ShowcaseSection
              title="ToolCallCard Component"
              description="Compact tool call display for message history (components/chat/tool-call-card.tsx)"
            >
              <ShowcaseItem label="Completed with Result">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.listFiles.toolCall}
                    toolResult={sampleToolCallMessages.listFiles.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Completed with Long Result (Expandable)">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.bashCommand.toolCall}
                    toolResult={sampleToolCallMessages.bashCommand.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Executing">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.executing.toolCall}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Error">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.error.toolCall}
                    toolResult={sampleToolCallMessages.error.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="write_todos Tool (Special Rendering)">
                <div className="pl-[25px]">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.writeTodos.toolCall}
                    toolResult={sampleToolCallMessages.writeTodos.toolResult}
                  />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* TodoListRenderer Section */}
            <ShowcaseSection
              title="TodoListRenderer Component"
              description="Task list renderer for write_todos tool (components/chat/todo-list-renderer.tsx)"
            >
              <ShowcaseItem label="Executing (Updating)">
                <TodoListRenderer
                  arguments={sampleTodoData.executing.arguments}
                  isExecuting={sampleTodoData.executing.isExecuting}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Completed (All Done)">
                <TodoListRenderer
                  arguments={sampleTodoData.completed.arguments}
                  result={sampleTodoData.completed.result}
                  isExecuting={sampleTodoData.completed.isExecuting}
                />
              </ShowcaseItem>

              <ShowcaseItem label="With Warning">
                <TodoListRenderer
                  arguments={sampleTodoData.withWarning.arguments}
                  result={sampleTodoData.withWarning.result}
                  isExecuting={sampleTodoData.withWarning.isExecuting}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Error State">
                <TodoListRenderer
                  arguments={sampleTodoData.error.arguments}
                  error={sampleTodoData.error.error}
                  isExecuting={sampleTodoData.error.isExecuting}
                />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Combined Chat View */}
            <ShowcaseSection
              title="Combined Chat View"
              description="Example conversation showing how components work together in Session UI"
            >
              <ShowcaseItem label="Full Conversation">
                <div className="space-y-4">
                  <UserMessage content="Can you list the files in my project and run the tests?" />
                  <AssistantMessage content="I'll check the project structure and run the test suite for you." />
                  <div className="pl-[25px] space-y-2">
                    <ToolCallCard
                      toolCall={sampleToolCallMessages.listFiles.toolCall}
                      toolResult={sampleToolCallMessages.listFiles.toolResult}
                    />
                    <ToolCallCard
                      toolCall={sampleToolCallMessages.bashCommand.toolCall}
                      toolResult={sampleToolCallMessages.bashCommand.toolResult}
                    />
                  </div>
                  <AssistantMessage content="Your project has 5 files and all 24 tests passed successfully." />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Image Attachments Section */}
            <ShowcaseSection
              title="Image Attachments"
              description="Components for uploading and displaying image attachments in chat (components/chat/image-attachments.tsx)"
            >
              <ShowcaseItem label="Pending Images (Upload Status)">
                <ImageAttachments
                  images={samplePendingImages}
                  onRemove={(tempId) => console.log("Remove:", tempId)}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Uploaded Image Only">
                <ImageAttachments
                  images={[samplePendingImages[0]]}
                  onRemove={(tempId) => console.log("Remove:", tempId)}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Uploading State">
                <ImageAttachments
                  images={[samplePendingImages[1]]}
                  onRemove={(tempId) => console.log("Remove:", tempId)}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Error State">
                <ImageAttachments
                  images={[samplePendingImages[2]]}
                  onRemove={(tempId) => console.log("Remove:", tempId)}
                />
              </ShowcaseItem>

              <ShowcaseItem label="MessageImage (In Chat History)">
                <div className="flex gap-2">
                  <MessageImage imageId="sample-image-1" filename="screenshot.png" />
                  <MessageImage imageId="sample-image-2" filename="diagram.png" />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="User Message with Image">
                <div className="flex justify-end">
                  <div className="max-w-[90%] bg-gray-500 text-white rounded-lg p-3">
                    <div className="flex items-start gap-2">
                      <div className="flex-1 space-y-2">
                        <p className="text-sm whitespace-pre-wrap">Here is a screenshot of the error I&apos;m seeing.</p>
                        <div className="flex flex-wrap gap-2 mt-2">
                          <div className="w-20 h-20 rounded-md overflow-hidden bg-white/10 flex items-center justify-center">
                            <span className="text-[10px] text-white/60">Preview</span>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Token Usage Display Section */}
            <ShowcaseSection
              title="Token Usage Display"
              description="Components showing LLM token usage statistics for agents and sessions"
            >
              <ShowcaseItem label="Usage Card (Agent Detail Page)">
                <div className="grid grid-cols-3 gap-4">
                  <TokenUsageCard usage={sampleUsageData.small} title="Small Usage" />
                  <TokenUsageCard usage={sampleUsageData.medium} title="Medium Usage" />
                  <TokenUsageCard usage={sampleUsageData.large} title="Large Usage" />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Usage Badge (Session Header)">
                <div className="flex items-center gap-4">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Small:</span>
                    <TokenUsageBadge usage={sampleUsageData.small} />
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Medium:</span>
                    <TokenUsageBadge usage={sampleUsageData.medium} />
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Large:</span>
                    <TokenUsageBadge usage={sampleUsageData.large} />
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Session Header with Usage">
                <div className="border rounded-lg p-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <h2 className="text-lg font-bold">Example Session</h2>
                      <p className="text-sm text-muted-foreground">Started Jan 15, 2026, 10:30 AM</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <TokenUsageBadge usage={sampleUsageData.medium} />
                      <Badge variant="outline" className="gap-1">claude-sonnet-4</Badge>
                      <Badge variant="secondary">Ready</Badge>
                    </div>
                  </div>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
