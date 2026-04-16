"use client";

import {
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
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";

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

const monitorToolCalls: ToolCallContent[] = [
  {
    id: "tool-monitor-create",
    name: "spawn_background",
    arguments: {
      tool: "github_watch_pr",
      title: "Watch PR 1319",
      schedule: {
        cron_expression: "*/10 * * * *",
        timezone: "America/Chicago",
      },
    },
  },
  {
    id: "tool-monitor-delete",
    name: "cancel_schedule",
    arguments: {
      schedule_id: "sched_0195monitor",
    },
  },
];

const monitorToolResults = new Map<string, ToolCompletedData>([
  [
    "tool-monitor-create",
    {
      tool_call_id: "tool-monitor-create",
      tool_name: "spawn_background",
      success: true,
      status: "success",
      result: [
        {
          type: "text",
          text: JSON.stringify({
            created: true,
            status: "scheduled",
            title: "Watch PR 1319",
            cron_expression: "*/10 * * * *",
            timezone: "America/Chicago",
          }),
        },
      ],
      duration_ms: 84,
    },
  ],
  [
    "tool-monitor-delete",
    {
      tool_call_id: "tool-monitor-delete",
      tool_name: "cancel_schedule",
      success: true,
      status: "success",
      result: [
        {
          type: "text",
          text: JSON.stringify({
            cancelled: true,
            description: `This scheduled monitor fired. Start the background run now.

Use \`spawn_background\` with:
- tool: \`github_watch_pr\`
- title: \`Watch PR 1319\`
- signal_on_completion: true
- args:
\`\`\`json
{"pull_request":1319}
\`\`\``,
          }),
        },
      ],
      duration_ms: 41,
    },
  ],
]);

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
        {
          type: "text",
          text: "Can you inspect this project and sketch the rewrite steps?",
        },
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
      content: [
        {
          type: "text",
          text: "Rewrite it so it supports a rock collection instead.",
        },
      ],
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
  return (
    <DevPageShell
      eyebrow="Tool Activity"
      title="Minimal execution states"
      description="Preview the same quieter transcript treatment used by the runtime chat."
      widthClassName="max-w-6xl"
    >
      <div className="mx-auto max-w-5xl">
        <div className="overflow-hidden border border-border bg-card">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-5 py-4">
            <div>
              <div className="text-[10px] uppercase tracking-[0.24em] text-muted-foreground">
                Everruns Chat
              </div>
              <div className="mt-1 text-sm font-medium text-foreground">Tool activity preview</div>
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
                  <div className={chatSurfaceStyles.userMessage}>
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

                <div className={chatSurfaceStyles.agentMessageRow}>
                  <div className={chatSurfaceStyles.agentIcon}>
                    <Bot className="h-3.5 w-3.5" />
                  </div>
                  <div className="flex-1 space-y-3">
                    <div className="flex items-start gap-2">
                      <p className={chatSurfaceStyles.agentMessage}>
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
                  <div className={chatSurfaceStyles.userMessage}>
                    <div className="mb-1 flex items-center justify-end">
                      <MessageInfoIcon event={rewriteInputEvent} />
                    </div>
                    Rewrite it so it supports a rock collection instead.
                  </div>
                </div>

                <div className={chatSurfaceStyles.agentMessageRow}>
                  <div className={chatSurfaceStyles.agentIcon}>
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

                <div className={chatSurfaceStyles.agentMessageRow}>
                  <div className={chatSurfaceStyles.agentIcon}>
                    <Bot className="h-3.5 w-3.5" />
                  </div>
                  <div className="flex-1 space-y-3">
                    <p className={chatSurfaceStyles.agentMessage}>
                      I set a monitor for the PR and removed it after merge.
                    </p>
                    <ToolActivityGroup
                      toolCalls={monitorToolCalls}
                      toolResultsMap={monitorToolResults}
                    />
                  </div>
                </div>

                <div className={chatSurfaceStyles.agentMessageRow}>
                  <div className={chatSurfaceStyles.agentIcon}>
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

              <div className={chatSurfaceStyles.composerSection}>
                <div className="mb-3 flex flex-wrap gap-2">
                  <div className="border border-border/70 bg-card/85 px-2 py-1 text-xs text-muted-foreground shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
                    architecture.png
                  </div>
                  <div className="border border-border/70 bg-card/85 px-2 py-1 text-xs text-muted-foreground shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
                    schema-draft.png
                  </div>
                </div>

                <div className={chatSurfaceStyles.composerInputShell}>
                  <div className="absolute bottom-full left-0 right-0 border border-border/70 bg-card/95 p-1 shadow-[0_1px_0_hsl(var(--background)/0.9)]">
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

                  <div className="px-4 py-3.5 text-sm text-foreground">
                    Type a message or <span className="font-mono">/</span> for commands...
                  </div>
                </div>

                <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
                  <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                    <button
                      type="button"
                      className={`inline-flex items-center justify-center ${chatSurfaceStyles.composerIconButton}`}
                      aria-label="Attach images"
                    >
                      <ImagePlus className="icon-sharp h-4 w-4" />
                    </button>
                    <div className={chatSurfaceStyles.composerControlChip}>
                      <span className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                        Model
                      </span>
                      <span className="text-sm text-foreground">Kimi K2.5</span>
                    </div>
                    <div className={chatSurfaceStyles.composerControlChip}>
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
                      className={`inline-flex items-center justify-center ${chatSurfaceStyles.composerDangerButton}`}
                      aria-label="Cancel current turn"
                    >
                      <StopCircle className="icon-sharp h-4 w-4" />
                    </button>
                    <button
                      type="button"
                      className={`inline-flex items-center justify-center ${chatSurfaceStyles.composerSubmitButton} bg-primary text-primary-foreground`}
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
    </DevPageShell>
  );
}
