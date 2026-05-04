"use client";

import type { ToolOutputStreams } from "@/app/(main)/sessions/[sessionId]/session-context";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";
import type { TimelineToolRow } from "@/components/chat/tool-activity-timeline-group";
import type { ToolCompletedData, ToolProgressData } from "@/lib/api/types";

function textResult(text: string): ToolCompletedData["result"] {
  return [{ type: "text", text }];
}

function makeCompletedResult({
  toolCallId,
  toolName,
  text,
  durationMs,
  error,
  success = true,
}: {
  toolCallId: string;
  toolName: string;
  text: string;
  durationMs?: number;
  error?: string;
  success?: boolean;
}): ToolCompletedData {
  return {
    tool_call_id: toolCallId,
    tool_name: toolName,
    success,
    status: success ? "success" : "error",
    result: textResult(text),
    error,
    duration_ms: durationMs,
  };
}

export const bashToolCall: ToolCallContent = {
  id: "tool-bash-showcase",
  name: "bash",
  arguments: { command: "pwd && uname -s" },
};

export const bashToolResult = makeCompletedResult({
  toolCallId: bashToolCall.id,
  toolName: bashToolCall.name,
  text: JSON.stringify(
    {
      cwd: "/home/daytona",
      stdout: "/home/daytona\nLinux\n",
      stderr: "",
      exit_code: 0,
      success: true,
    },
    null,
    2,
  ),
  durationMs: 180,
});

export const bashToolOutput: ToolOutputStreams = {
  stdout: "/home/daytona\nLinux\n",
  stderr: "",
};

export const readFileToolCall: ToolCallContent = {
  id: "tool-read-showcase",
  name: "read_file",
  arguments: {
    path: "/workspace/apps/ui/src/components/chat/tool-call-chrome.ts",
  },
};

export const readFileToolResult = makeCompletedResult({
  toolCallId: readFileToolCall.id,
  toolName: readFileToolCall.name,
  text: JSON.stringify(
    {
      path: "/workspace/apps/ui/src/components/chat/tool-call-chrome.ts",
      encoding: "text",
      size_bytes: 1240,
      content: "export const toolCallChrome = { card: 'border border-border/70 bg-card/90', ... }",
    },
    null,
    2,
  ),
  durationMs: 42,
});

export const writeFileToolCall: ToolCallContent = {
  id: "tool-write-showcase",
  name: "write_file",
  arguments: {
    path: "/workspace/apps/ui/src/components/chat/tool-call-card.tsx",
  },
};

export const writeFileToolResult = makeCompletedResult({
  toolCallId: writeFileToolCall.id,
  toolName: writeFileToolCall.name,
  text: JSON.stringify(
    {
      path: "/workspace/apps/ui/src/components/chat/tool-call-card.tsx",
      created: false,
      size_bytes: 2134,
      applied_edits: 3,
      first_changed_line: 24,
    },
    null,
    2,
  ),
  durationMs: 94,
});

export const todoToolCall: ToolCallContent = {
  id: "tool-todos-showcase",
  name: "write_todos",
  arguments: {
    todos: [
      {
        content: "Audit current transcript components",
        activeForm: "Auditing current transcript components",
        status: "completed",
      },
      {
        content: "Replace dev mocks with production session chrome",
        activeForm: "Replacing dev mocks with production session chrome",
        status: "in_progress",
      },
      {
        content: "Verify the new showcase pages",
        activeForm: "Verifying the new showcase pages",
        status: "pending",
      },
    ],
  },
};

export const todoToolResult = makeCompletedResult({
  toolCallId: todoToolCall.id,
  toolName: todoToolCall.name,
  text: "todo list updated",
  durationMs: 26,
});

export const groupedActivityToolCalls: ToolCallContent[] = [
  {
    id: "tool-list-files",
    name: "list_files",
    arguments: { path: "." },
  },
  {
    id: "tool-search-web",
    name: "search_web",
    arguments: { query: "daytona sandbox auth issue" },
  },
  {
    id: "tool-edit-file",
    name: "edit_file",
    arguments: {
      path: "/workspace/apps/ui/src/components/chat/tool-call-card.tsx",
    },
  },
];

export const groupedActivityResults = new Map<string, ToolCompletedData>([
  [
    "tool-list-files",
    makeCompletedResult({
      toolCallId: "tool-list-files",
      toolName: "list_files",
      text: "apps/\ncrates/\nscripts/\nAGENTS.md",
      durationMs: 54,
    }),
  ],
  [
    "tool-search-web",
    makeCompletedResult({
      toolCallId: "tool-search-web",
      toolName: "search_web",
      text: "Auth bootstrap and middleware ordering can cause login redirects in dev.",
      durationMs: 260,
    }),
  ],
  [
    "tool-edit-file",
    makeCompletedResult({
      toolCallId: "tool-edit-file",
      toolName: "edit_file",
      text: "diff applied",
      durationMs: 120,
    }),
  ],
]);

export const groupedActivityProgress = new Map<string, ToolProgressData>([
  [
    "tool-search-web",
    {
      tool_call_id: "tool-search-web",
      tool_name: "search_web",
      message: "Comparing docs and UI behavior",
    },
  ],
]);

export const timelineRows: TimelineToolRow[] = [
  {
    id: "timeline-create-sandbox",
    label: "Created Daytona sandbox",
    state: "completed",
    result: makeCompletedResult({
      toolCallId: "timeline-create-sandbox",
      toolName: "daytona_create_sandbox",
      text: JSON.stringify(
        {
          sandbox_id: "9beac0d5-ae4c-41e8-970f-f48e70a20395",
          status: "running",
          workspace_path: "/home/daytona",
        },
        null,
        2,
      ),
      durationMs: 620,
    }),
  },
  {
    id: "timeline-run-check",
    label: "Ran `pwd; uname -s`",
    state: "error",
    result: makeCompletedResult({
      toolCallId: "timeline-run-check",
      toolName: "daytona_exec",
      text: "Resource was not created by this session",
      durationMs: 180,
      error: "Resource was not created by this session",
      success: false,
    }),
  },
  {
    id: "timeline-read-file",
    label: "Read the file back",
    state: "completed",
    result: makeCompletedResult({
      toolCallId: "timeline-read-file",
      toolName: "daytona_exec",
      text: "hello from dev.everruns.com",
      durationMs: 120,
    }),
  },
];
