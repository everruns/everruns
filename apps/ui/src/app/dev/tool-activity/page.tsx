"use client";

import Link from "next/link";
import {
  ArrowLeft,
  Bot,
  Brain,
  CalendarClock,
  ImagePlus,
  Send,
  Sparkles,
  StopCircle,
  Terminal,
} from "lucide-react";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import type { Event, ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";

const isDev = process.env.NODE_ENV === "development";

const activeToolCalls: ToolCallContent[] = [
  {
    id: "tool-list",
    name: "list_files",
    arguments: { path: "." },
  },
  {
    id: "tool-read",
    name: "read_file",
    arguments: { path: "/workspace/README.md" },
  },
  {
    id: "tool-grep",
    name: "grep_files",
    arguments: { pattern: "**/package.json" },
  },
  {
    id: "tool-search",
    name: "search_web",
    arguments: { query: "next.js structured tool activity ui" },
  },
];

const activeToolResults = new Map<string, ToolCompletedData>([
  [
    "tool-list",
    {
      tool_call_id: "tool-list",
      tool_name: "list_files",
      success: true,
      status: "success",
      result: [
        {
          type: "text",
          text: "README.md\npackage.json\nsrc/\napps/\ncrates/",
        },
      ],
    },
  ],
  [
    "tool-read",
    {
      tool_call_id: "tool-read",
      tool_name: "read_file",
      success: true,
      status: "success",
      result: [
        {
          type: "text",
          text: JSON.stringify({
            path: "/workspace/README.md",
            content: "# Everruns\nDurable agent runtime and UI for long-running tasks.",
            encoding: "text",
            size_bytes: 63,
          }),
        },
      ],
    },
  ],
]);

const completedToolCalls: ToolCallContent[] = [
  {
    id: "tool-shell",
    name: "bash",
    arguments: { command: "cargo test -p everruns-ui" },
  },
  {
    id: "tool-edit",
    name: "write_file",
    arguments: { path: "/workspace/README.md" },
  },
];

const completedToolResults = new Map<string, ToolCompletedData>([
  [
    "tool-shell",
    {
      tool_call_id: "tool-shell",
      tool_name: "bash",
      success: false,
      status: "error",
      result: [
        {
          type: "text",
          text: JSON.stringify({
            stdout: "running 4 tests\n3 passed",
            stderr: "thread 'tool_activity' panicked at assertion failed: left == right",
            exit_code: 101,
            success: false,
          }),
        },
      ],
      duration_ms: 910,
    },
  ],
  [
    "tool-edit",
    {
      tool_call_id: "tool-edit",
      tool_name: "write_file",
      success: true,
      status: "success",
      result: [
        {
          type: "text",
          text: JSON.stringify({
            path: "/workspace/README.md",
            size_bytes: 1584,
            created: true,
          }),
        },
      ],
      duration_ms: 95,
    },
  ],
]);

const clientToolCalls: ToolCallContent[] = [
  {
    id: "tool-picker",
    name: "pick_file",
    display_name: "Pick File",
    arguments: { accept: ".png,.jpg" },
  },
];

const planTodos = [
  {
    content: "Update schema for rock collection domain",
    activeForm: "Updating schema for rock collection domain",
    status: "completed" as const,
  },
  {
    content: "Rename Convex functions and routes",
    activeForm: "Renaming Convex functions and routes",
    status: "completed" as const,
  },
  {
    content: "Update UI copy for rock collection",
    activeForm: "Updating UI copy for rock collection",
    status: "in_progress" as const,
  },
  {
    content: "Refresh README examples",
    activeForm: "Refreshing README examples",
    status: "pending" as const,
  },
];

const scheduledInputEvent: Event = {
  id: "evt-user-1",
  type: "input.message",
  ts: "2026-03-07T21:20:00Z",
  session_id: "session-dev-tool-activity",
  context: {},
  data: {
    message: {
      id: "msg-user-1",
      session_id: "session-dev-tool-activity",
      sequence: 1,
      role: "user",
      content: [
        { type: "text", text: "Can you inspect this project and sketch the rewrite steps?" },
      ],
      tool_call_id: null,
      created_at: "2026-03-07T21:20:00Z",
      metadata: { source: "schedule" },
    },
  },
};

const rewriteInputEvent: Event = {
  id: "evt-user-2",
  type: "input.message",
  ts: "2026-03-07T21:21:00Z",
  session_id: "session-dev-tool-activity",
  context: {},
  data: {
    message: {
      id: "msg-user-2",
      session_id: "session-dev-tool-activity",
      sequence: 2,
      role: "user",
      content: [{ type: "text", text: "Rewrite it so it supports a rock collection instead." }],
      tool_call_id: null,
      created_at: "2026-03-07T21:21:00Z",
    },
  },
};

const agentPlanningEvent: Event = {
  id: "evt-agent-1",
  type: "output.message.completed",
  ts: "2026-03-07T21:20:08Z",
  session_id: "session-dev-tool-activity",
  context: {},
  data: {
    message: {
      id: "msg-agent-1",
      session_id: "session-dev-tool-activity",
      sequence: 3,
      role: "agent",
      content: [
        {
          type: "text",
          text: "I'm checking the codebase shape first, then I'll summarize the likely rewrite path.",
        },
      ],
      tool_call_id: null,
      created_at: "2026-03-07T21:20:08Z",
      metadata: {
        model: "kimi-k2.5",
        reasoning_effort: "medium",
      },
    },
    metadata: { model: "kimi-k2.5" },
    usage: { input_tokens: 1220, output_tokens: 188 },
  },
};

export default function ToolActivityDevPage() {
  if (!isDev) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="mt-2 text-muted-foreground">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background bg-brand-dots px-4 py-8">
      <div className="mx-auto max-w-6xl">
        <Link
          href="/dev"
          className="mb-6 inline-flex items-center gap-2 border border-border bg-card px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-muted/35 hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Developer Tools
        </Link>

        <div className="mb-8 max-w-2xl space-y-2">
          <p className="text-[11px] uppercase tracking-[0.3em] text-muted-foreground">
            Tool Activity
          </p>
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            Minimal execution states
          </h1>
          <p className="text-sm leading-6 text-muted-foreground">
            Sharper, quieter, closer to the final chat surface.
          </p>
        </div>

        <div className="mx-auto max-w-5xl">
          <div className="overflow-hidden border border-border bg-card">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-5 py-4">
              <div>
                <div className="text-[10px] uppercase tracking-[0.24em] text-muted-foreground">
                  Everruns Chat
                </div>
                <div className="mt-1 text-sm font-medium text-foreground">
                  Tool activity preview
                </div>
              </div>
              <div className="flex items-center gap-2 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                <span className="inline-flex h-2 w-2 bg-accent" />
                Preview
              </div>
            </div>

            <div className="min-h-[760px] bg-background/80">
              <div className="flex flex-col">
                <div className="flex-1 space-y-6 px-4 py-5 sm:px-6">
                  <div className="flex justify-end">
                    <div className="max-w-[78%] border-r-2 border-r-accent bg-[hsl(var(--accent)/0.1)] px-4 py-3 text-sm text-foreground">
                      <div className="mb-1 flex items-center justify-between gap-2">
                        <div className="flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                          <CalendarClock className="h-3 w-3" />
                          Scheduled
                        </div>
                        <MessageInfoIcon event={scheduledInputEvent} />
                      </div>
                      Can you inspect this project and sketch the rewrite steps?
                    </div>
                  </div>

                  <div className="flex items-start gap-3">
                    <div className="mt-1 flex h-6 w-6 items-center justify-center border border-border bg-primary text-primary-foreground">
                      <Bot className="h-3.5 w-3.5" />
                    </div>
                    <div className="flex-1 space-y-3">
                      <div className="flex items-start gap-2">
                        <p className="flex-1 border-l-2 border-l-primary bg-card px-4 py-3 text-sm leading-7 text-foreground">
                          I&apos;m checking the codebase shape first, then I&apos;ll summarize the
                          likely rewrite path.
                        </p>
                        <MessageInfoIcon event={agentPlanningEvent} />
                      </div>
                      <ToolActivityGroup
                        toolCalls={activeToolCalls}
                        toolResultsMap={activeToolResults}
                      />
                    </div>
                  </div>

                  <div className="flex justify-end">
                    <div className="max-w-[78%] border-r-2 border-r-accent bg-[hsl(var(--accent)/0.1)] px-4 py-3 text-sm text-foreground">
                      <div className="mb-1 flex items-center justify-end">
                        <MessageInfoIcon event={rewriteInputEvent} />
                      </div>
                      Rewrite it so it supports a rock collection instead.
                    </div>
                  </div>

                  <div className="flex items-start gap-3">
                    <div className="mt-1 flex h-6 w-6 items-center justify-center border border-border bg-primary text-primary-foreground">
                      <Bot className="h-3.5 w-3.5" />
                    </div>
                    <div className="flex-1 space-y-3">
                      <ToolActivityGroup
                        toolCalls={completedToolCalls}
                        toolResultsMap={completedToolResults}
                      />
                      <TodoListRenderer arguments={{ todos: planTodos }} isExecuting />
                    </div>
                  </div>

                  <div className="flex items-start gap-3">
                    <div className="mt-1 flex h-6 w-6 items-center justify-center border border-border bg-primary text-primary-foreground">
                      <Bot className="h-3.5 w-3.5" />
                    </div>
                    <div className="flex-1">
                      <ToolActivityGroup
                        toolCalls={clientToolCalls}
                        toolResultsMap={new Map()}
                        mode="client"
                      />
                    </div>
                  </div>
                </div>

                <div className="border-t border-border bg-muted/30 p-4">
                  <div className="mb-3 flex flex-wrap gap-2">
                    <div className="border border-border bg-background px-2 py-1 text-xs text-muted-foreground">
                      architecture.png
                    </div>
                    <div className="border border-border bg-background px-2 py-1 text-xs text-muted-foreground">
                      schema-draft.png
                    </div>
                  </div>

                  <div className="relative border border-border bg-background">
                    <div className="absolute bottom-full left-0 right-0 border border-border bg-card p-1">
                      <button
                        type="button"
                        className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm text-foreground"
                      >
                        <Terminal className="h-3.5 w-3.5 text-muted-foreground" />
                        <span className="font-mono text-xs">/ship</span>
                        <span className="truncate text-xs text-muted-foreground">
                          Run the shipping workflow
                        </span>
                      </button>
                      <button
                        type="button"
                        className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm text-muted-foreground"
                      >
                        <Sparkles className="h-3.5 w-3.5 text-muted-foreground" />
                        <span className="font-mono text-xs">/process-issues</span>
                        <span className="truncate text-xs text-muted-foreground">
                          Batch-process open issues
                        </span>
                      </button>
                    </div>

                    <div className="px-4 py-5 text-sm text-foreground">
                      Type a message or <span className="font-mono">/</span> for commands...
                    </div>
                  </div>

                  <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
                    <div className="flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                      <button
                        type="button"
                        className="inline-flex h-10 w-10 items-center justify-center border border-border bg-background"
                        aria-label="Attach images"
                      >
                        <ImagePlus className="icon-sharp h-4 w-4" />
                      </button>
                      <div className="inline-flex h-10 items-center gap-2 border border-border bg-background px-3">
                        <span className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                          Model
                        </span>
                        <span className="text-sm text-foreground">Kimi K2.5</span>
                      </div>
                      <div className="inline-flex h-10 items-center gap-2 border border-border bg-background px-3">
                        <Brain className="icon-sharp h-3.5 w-3.5" />
                        <span className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                          Reasoning
                        </span>
                        <span className="text-sm text-foreground">Default</span>
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        className="inline-flex h-10 w-10 items-center justify-center border border-destructive/30 bg-destructive/[0.08] text-destructive"
                        aria-label="Cancel current turn"
                      >
                        <StopCircle className="icon-sharp h-4 w-4" />
                      </button>
                      <button
                        type="button"
                        className="inline-flex h-10 w-10 items-center justify-center bg-primary text-primary-foreground"
                        aria-label="Send message"
                      >
                        <Send className="icon-sharp h-4 w-4" />
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
