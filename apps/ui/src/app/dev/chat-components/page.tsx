"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Bot, ArrowLeft } from "lucide-react";
import { ToolCallCard } from "@/components/chat/tool-call-card";
import { ToolCallCardFromEvent } from "@/components/chat/tool-call-card-from-event";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ImageAttachments, MessageImage } from "@/components/chat/image-attachments";
import type { Message, Event, ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";
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

function ShowcaseItem({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <div className="text-sm font-medium text-muted-foreground">{label}</div>
      <div className="border rounded-lg p-4 bg-background">{children}</div>
    </div>
  );
}

// ============================================
// Message Rendering (Minimal + Icon style)
// These match the new styling in sessions/[sessionId]/chat/page.tsx
// ============================================

function UserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] border border-border/60 rounded-xl px-3 py-2">
        <p className="text-sm whitespace-pre-wrap">{content}</p>
      </div>
    </div>
  );
}

function AssistantMessage({ content }: { content: string }) {
  return (
    <div className="w-full flex items-start gap-2">
      <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground/60" />
      <p className="text-sm whitespace-pre-wrap text-foreground/90">{content}</p>
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
      content: [
        {
          type: "tool_call" as const,
          id: "tc-1",
          name: "list_files",
          arguments: { path: "/home/user/project", recursive: true },
        },
      ],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-1",
      session_id: "session-1",
      sequence: 6,
      role: "tool_result" as const,
      content: [
        {
          type: "tool_result" as const,
          tool_call_id: "tc-1",
          result: ["src/", "src/main.rs", "src/lib.rs", "Cargo.toml", "README.md"],
        },
      ],
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
      content: [
        {
          type: "tool_call" as const,
          id: "tc-2",
          name: "bash",
          arguments: { command: "cargo test --workspace" },
        },
      ],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-2",
      session_id: "session-1",
      sequence: 8,
      role: "tool_result" as const,
      content: [
        {
          type: "tool_result" as const,
          tool_call_id: "tc-2",
          result:
            "running 24 tests\ntest storage::tests::test_create_agent ... ok\ntest storage::tests::test_list_agents ... ok\ntest api::tests::test_health_endpoint ... ok\ntest api::tests::test_create_session ... ok\n\ntest result: ok. 24 passed; 0 failed; 0 ignored",
        },
      ],
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
      content: [
        {
          type: "tool_call" as const,
          id: "tc-3",
          name: "read_file",
          arguments: { path: "/home/user/project/src/main.rs" },
        },
      ],
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
      content: [
        {
          type: "tool_call" as const,
          id: "tc-4",
          name: "write_file",
          arguments: { path: "/etc/protected/config.json", content: "{}" },
        },
      ],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-4",
      session_id: "session-1",
      sequence: 11,
      role: "tool_result" as const,
      content: [
        {
          type: "tool_result" as const,
          tool_call_id: "tc-4",
          error: "Permission denied: Cannot write to /etc/protected/config.json",
        },
      ],
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
      content: [
        {
          type: "tool_call" as const,
          id: "tc-5",
          name: "write_todos",
          arguments: {
            todos: [
              {
                content: "Review code changes",
                activeForm: "Reviewing code changes",
                status: "completed",
              },
              { content: "Run tests", activeForm: "Running tests", status: "in_progress" },
              {
                content: "Update documentation",
                activeForm: "Updating documentation",
                status: "pending",
              },
              {
                content: "Create pull request",
                activeForm: "Creating pull request",
                status: "pending",
              },
            ],
          },
        },
      ],
      tool_call_id: null,
      created_at: new Date().toISOString(),
    },
    toolResult: {
      id: "msg-tr-5",
      session_id: "session-1",
      sequence: 13,
      role: "tool_result" as const,
      content: [
        {
          type: "tool_result" as const,
          tool_call_id: "tc-5",
          result: {
            success: true,
            total_tasks: 4,
            pending: 2,
            in_progress: 1,
            completed: 1,
            todos: [
              {
                content: "Review code changes",
                activeForm: "Reviewing code changes",
                status: "completed",
              },
              { content: "Run tests", activeForm: "Running tests", status: "in_progress" },
              {
                content: "Update documentation",
                activeForm: "Updating documentation",
                status: "pending",
              },
              {
                content: "Create pull request",
                activeForm: "Creating pull request",
                status: "pending",
              },
            ],
          },
        },
      ],
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
        {
          content: "Analyze requirements",
          activeForm: "Analyzing requirements",
          status: "completed",
        },
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
        {
          content: "Create API endpoints",
          activeForm: "Creating API endpoints",
          status: "completed",
        },
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
        {
          content: "Create API endpoints",
          activeForm: "Creating API endpoints",
          status: "completed",
        },
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

// Sample image attachment data
const samplePendingImages: PendingImage[] = [
  {
    tempId: "temp-1",
    file: null,
    uploadPromise: null,
    imageId: "img-uploaded-1",
    filename: "screenshot.png",
    previewUrl:
      "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23e2e8f0' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%2394a3b8' font-size='10'%3EPNG%3C/text%3E%3C/svg%3E",
    status: "uploaded",
  },
  {
    tempId: "temp-2",
    file: new File([], "photo.jpg"),
    uploadPromise: Promise.resolve({
      id: "",
      filename: "",
      content_type: "",
      size_bytes: 0,
      created_at: "",
    }),
    imageId: null,
    filename: "photo.jpg",
    previewUrl:
      "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23fef3c7' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%23d97706' font-size='10'%3EJPG%3C/text%3E%3C/svg%3E",
    status: "uploading",
  },
  {
    tempId: "temp-3",
    file: null,
    uploadPromise: null,
    imageId: null,
    filename: "failed.gif",
    previewUrl:
      "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23fee2e2' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%23dc2626' font-size='10'%3EGIF%3C/text%3E%3C/svg%3E",
    status: "error",
    error: "Upload failed",
  },
];

// Sample data for BashToolCallCard (event-based)
const sampleBashEventData: Record<
  string,
  { toolCall: ToolCallContent; toolResult?: ToolCompletedData }
> = {
  // Successful command with stdout only
  success: {
    toolCall: {
      id: "tc-bash-1",
      name: "bash",
      arguments: {
        command: "cargo test --workspace",
        description: "Run all workspace tests",
      },
    },
    toolResult: {
      tool_call_id: "tc-bash-1",
      tool_name: "bash",
      success: true,
      status: "success",
      result: [
        {
          type: "text" as const,
          text: JSON.stringify({
            stdout:
              "running 24 tests\ntest storage::tests::test_create_agent ... ok\ntest storage::tests::test_list_agents ... ok\ntest api::tests::test_health_endpoint ... ok\ntest api::tests::test_create_session ... ok\n\ntest result: ok. 24 passed; 0 failed; 0 ignored",
            stderr: "",
            exit_code: 0,
            success: true,
          }),
        },
      ],
      duration_ms: 4523,
    },
  },
  // Failed command with stderr
  withStderr: {
    toolCall: {
      id: "tc-bash-2",
      name: "bash",
      arguments: {
        command: "cargo clippy -- -D warnings",
        description: "Run clippy linter",
      },
    },
    toolResult: {
      tool_call_id: "tc-bash-2",
      tool_name: "bash",
      success: true,
      status: "success",
      result: [
        {
          type: "text" as const,
          text: JSON.stringify({
            stdout: "    Checking everruns-core v0.1.0\n",
            stderr:
              "error[E0308]: mismatched types\n  --> src/lib.rs:42:5\n   |\n42 |     let x: u32 = \"hello\";\n   |                  ^^^^^^^ expected `u32`, found `&str`\n\nerror: aborting due to 1 previous error",
            exit_code: 1,
            success: false,
          }),
        },
      ],
      duration_ms: 12340,
    },
  },
  // Currently executing
  executing: {
    toolCall: {
      id: "tc-bash-3",
      name: "bash",
      arguments: {
        command: "npm run build",
      },
    },
  },
  // Simple short command
  shortOutput: {
    toolCall: {
      id: "tc-bash-4",
      name: "bash",
      arguments: { command: "echo hello" },
    },
    toolResult: {
      tool_call_id: "tc-bash-4",
      tool_name: "bash",
      success: true,
      status: "success",
      result: [
        {
          type: "text" as const,
          text: JSON.stringify({
            stdout: "hello\n",
            stderr: "",
            exit_code: 0,
            success: true,
          }),
        },
      ],
      duration_ms: 45,
    },
  },
};

// Sample event data for MessageInfoIcon
const sampleEvents = {
  userMessage: {
    id: "evt-user-123e4567-e89b-12d3-a456-426614174000",
    type: "input.message",
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
    type: "output.message.completed",
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
};

// ============================================
// Main Page Component
// ============================================

export default function DevChatComponentsPage() {
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
          <h1 className="text-3xl font-bold">Chat Components</h1>
          <p className="text-muted-foreground mt-2">
            Chat-specific components: messages, tool calls, todo lists, and attachments
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <ScrollArea className="h-[calc(100vh-12rem)]">
          <div className="space-y-8 pr-4">
            {/* Message Rendering Section */}
            <ShowcaseSection
              title="Message Rendering (Minimal + Icon Style)"
              description="Current message styles used in the Chat UI"
            >
              <ShowcaseItem label="User Message">
                <UserMessage content="Hello! Can you help me analyze this code?" />
              </ShowcaseItem>

              <ShowcaseItem label="User Message (Long)">
                <UserMessage content="I need to refactor the authentication system to support OAuth 2.0 in addition to the existing session-based auth. The new system should maintain backward compatibility." />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message">
                <AssistantMessage content="I'll help you with that. Let me start by examining the current authentication implementation." />
              </ShowcaseItem>

              <ShowcaseItem label="Assistant Message (Multiline)">
                <AssistantMessage
                  content={
                    "Here's my analysis:\n\n1. Current auth uses session cookies\n2. User model has email/password fields\n3. No OAuth support exists yet\n\nI recommend starting with the OAuth provider abstraction."
                  }
                />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* MessageInfoIcon Section */}
            <ShowcaseSection
              title="MessageInfoIcon Component"
              description="Small info icon showing message metadata on hover"
            >
              <ShowcaseItem label="User Message with Info (Light Variant)">
                <div className="flex justify-end">
                  <div className="max-w-[85%] border border-border/60 rounded-xl px-3 py-2">
                    <div className="flex items-start gap-2">
                      <p className="text-sm whitespace-pre-wrap flex-1">Hello! Can you help me?</p>
                      <MessageInfoIcon event={sampleEvents.userMessage} variant="light" />
                    </div>
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Agent Message with Info">
                <div className="w-full flex items-start gap-2">
                  <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground/60" />
                  <div className="flex-1 flex items-start gap-2">
                    <p className="text-sm whitespace-pre-wrap flex-1 text-foreground/90">
                      Hi there! How can I help?
                    </p>
                    <MessageInfoIcon event={sampleEvents.agentMessage} />
                  </div>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* ToolCallCard Section */}
            <ShowcaseSection
              title="ToolCallCard Component"
              description="Compact tool call display with status and output toggle"
            >
              <ShowcaseItem label="Completed with Result">
                <div className="ml-6">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.listFiles.toolCall}
                    toolResult={sampleToolCallMessages.listFiles.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Completed with Long Result (Expandable)">
                <div className="ml-6">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.bashCommand.toolCall}
                    toolResult={sampleToolCallMessages.bashCommand.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Executing">
                <div className="ml-6">
                  <ToolCallCard toolCall={sampleToolCallMessages.executing.toolCall} />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Error">
                <div className="ml-6">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.error.toolCall}
                    toolResult={sampleToolCallMessages.error.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="write_todos Tool (Special Rendering)">
                <div className="ml-6">
                  <ToolCallCard
                    toolCall={sampleToolCallMessages.writeTodos.toolCall}
                    toolResult={sampleToolCallMessages.writeTodos.toolResult}
                  />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* BashToolCallCard Section */}
            <ShowcaseSection
              title="BashToolCallCard Component"
              description="Claude Code-style bash rendering: $ command, structured stdout/stderr"
            >
              <ShowcaseItem label="Successful Command (Collapsed)">
                <div className="ml-6">
                  <ToolCallCardFromEvent
                    toolCall={sampleBashEventData.success.toolCall}
                    toolResult={sampleBashEventData.success.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Failed Command with stderr">
                <div className="ml-6">
                  <ToolCallCardFromEvent
                    toolCall={sampleBashEventData.withStderr.toolCall}
                    toolResult={sampleBashEventData.withStderr.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Executing">
                <div className="ml-6">
                  <ToolCallCardFromEvent
                    toolCall={sampleBashEventData.executing.toolCall}
                    toolResult={sampleBashEventData.executing.toolResult}
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Short Output">
                <div className="ml-6">
                  <ToolCallCardFromEvent
                    toolCall={sampleBashEventData.shortOutput.toolCall}
                    toolResult={sampleBashEventData.shortOutput.toolResult}
                  />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* TodoListRenderer Section */}
            <ShowcaseSection
              title="TodoListRenderer Component"
              description="Compact task list renderer for write_todos tool"
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
              description="Example conversation showing how components work together"
            >
              <ShowcaseItem label="Full Conversation">
                <div className="space-y-4">
                  <UserMessage content="Can you list the files in my project and run the tests?" />
                  <AssistantMessage content="I'll check the project structure and run the test suite for you." />
                  <div className="ml-6 space-y-2">
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
              description="Components for uploading and displaying image attachments"
            >
              <ShowcaseItem label="Pending Images (Upload Status)">
                <ImageAttachments
                  images={samplePendingImages}
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
                  <div className="max-w-[85%] border border-border/60 rounded-xl px-3 py-2">
                    <div className="flex-1 space-y-2">
                      <p className="text-sm whitespace-pre-wrap">
                        Here is a screenshot of the error.
                      </p>
                      <div className="flex flex-wrap gap-2 mt-2">
                        <div className="w-20 h-20 rounded-md overflow-hidden bg-muted/50 flex items-center justify-center">
                          <span className="text-[10px] text-muted-foreground">Preview</span>
                        </div>
                      </div>
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
