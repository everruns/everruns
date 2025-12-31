"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Bot, ArrowLeft } from "lucide-react";
import { ToolCallCard } from "@/components/chat/tool-call-card";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import type { Message } from "@/lib/api/types";

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
      role: "assistant" as const,
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
      role: "assistant" as const,
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
      role: "assistant" as const,
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
      role: "assistant" as const,
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
      role: "assistant" as const,
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
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
