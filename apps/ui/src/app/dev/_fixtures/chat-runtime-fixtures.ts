"use client";

import type { PendingImage } from "@/lib/api/images";
import type {
  CommandDescriptor,
  Event,
  Model,
  ToolCompletedData,
  ToolProgressData,
} from "@/lib/api/types";
import type { ToolOutputStreams } from "@/app/(main)/sessions/[sessionId]/session-context";

export interface DevChatFixture {
  sessionId: string;
  events: Event[];
  toolResultsMap: Map<string, ToolCompletedData>;
  toolProgressMap: Map<string, ToolProgressData>;
  toolOutputMap: Map<string, ToolOutputStreams>;
  pendingImages: PendingImage[];
  commands: CommandDescriptor[];
  models: Model[];
  initialInputValue: string;
  initialModelId: string;
  initialReasoningEffort: string;
}

function makeInputEvent({
  id,
  sequence,
  sessionId,
  text,
  ts,
  metadata,
}: {
  id: string;
  sequence: number;
  sessionId: string;
  text: string;
  ts: string;
  metadata?: Record<string, unknown>;
}): Event {
  return {
    id,
    type: "input.message",
    ts,
    session_id: sessionId,
    context: {},
    data: {
      message: {
        id: `msg-${id}`,
        session_id: sessionId,
        sequence,
        role: "user",
        content: [{ type: "text", text }],
        tool_call_id: null,
        created_at: ts,
        metadata,
      },
    },
  };
}

function makeOutputEvent({
  id,
  sequence,
  sessionId,
  ts,
  text,
  toolCalls,
  metadata,
  context = {},
}: {
  id: string;
  sequence: number;
  sessionId: string;
  ts: string;
  text?: string;
  toolCalls?: Array<{
    id: string;
    name: string;
    arguments: Record<string, unknown>;
  }>;
  metadata?: Record<string, unknown>;
  context?: Event["context"];
}): Event {
  return {
    id,
    type: "output.message.completed",
    ts,
    session_id: sessionId,
    context,
    data: {
      message: {
        id: `msg-${id}`,
        session_id: sessionId,
        sequence,
        role: "agent",
        content: [
          ...(text ? [{ type: "text" as const, text }] : []),
          ...((toolCalls ?? []).map((toolCall) => ({
            type: "tool_call" as const,
            id: toolCall.id,
            name: toolCall.name,
            arguments: toolCall.arguments,
          })) ?? []),
        ],
        tool_call_id: null,
        created_at: ts,
        metadata,
      },
      metadata: { model: "kimi-k2.5" },
      usage: { input_tokens: 820, output_tokens: 140 },
    },
  };
}

function toolResult(
  tool_call_id: string,
  tool_name: string,
  result: ToolCompletedData["result"],
  extra?: Partial<ToolCompletedData>,
): ToolCompletedData {
  return {
    tool_call_id,
    tool_name,
    success: true,
    status: "success",
    result,
    ...extra,
  };
}

function dataPreviewSvg(label: string, fill: string, text: string): string {
  return `data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='96' height='96' viewBox='0 0 96 96'%3E%3Crect fill='${fill}' width='96' height='96'/%3E%3Ctext x='50%25' y='44%25' dominant-baseline='middle' text-anchor='middle' fill='%230a1636' font-size='11'%3E${label}%3C/text%3E%3Ctext x='50%25' y='60%25' dominant-baseline='middle' text-anchor='middle' fill='%230a1636' font-size='8'%3E${text}%3C/text%3E%3C/svg%3E`;
}

export const devCommands: CommandDescriptor[] = [
  {
    name: "ship",
    description: "Run the shipping workflow",
    source: "system",
  },
  {
    name: "process-issues",
    description: "Batch-process open issues",
    source: "skill",
  },
  {
    name: "btw",
    description: "Ask a side question without changing the main task",
    source: "system",
    args: [{ name: "question", description: "Question to answer", required: true }],
  },
];

export const devModels: Model[] = [
  {
    id: "model-kimi-k2.5",
    provider_id: "provider-openrouter",
    model_id: "openrouter/kimi-k2.5",
    display_name: "Kimi K2.5",
    capabilities: ["chat", "tools"],
    enabled: true,
    is_favorite: true,
    created_at: "2026-03-07T21:00:00Z",
    updated_at: "2026-03-07T21:00:00Z",
  },
  {
    id: "model-gpt-5.4-mini",
    provider_id: "provider-openai",
    model_id: "gpt-5.4-mini",
    display_name: "GPT-5.4 Mini",
    capabilities: ["chat", "tools"],
    enabled: true,
    is_favorite: false,
    created_at: "2026-03-07T21:00:00Z",
    updated_at: "2026-03-07T21:00:00Z",
  },
];

export function makePendingImages(): PendingImage[] {
  return [
    {
      tempId: "pending-layout",
      file: null,
      uploadPromise: null,
      imageId: null,
      filename: "layout.png",
      previewUrl: dataPreviewSvg("PNG", "%23ece7db", "layout"),
      status: "uploaded",
    },
    {
      tempId: "pending-sandbox",
      file: null,
      uploadPromise: null,
      imageId: null,
      filename: "sandbox-daytona.jpg",
      previewUrl: dataPreviewSvg("JPG", "%23f6e7c1", "sandbox"),
      status: "uploading",
    },
  ];
}

export function getDevChatFixture(kind: "tool-activity" | "chat-components"): DevChatFixture {
  if (kind === "tool-activity") {
    const sessionId = "session-dev-tool-activity";
    const events: Event[] = [
      makeInputEvent({
        id: "tool-user-1",
        sequence: 1,
        sessionId,
        text: "Inspect the repo and show me the path you took through the code.",
        ts: "2026-03-07T21:20:00Z",
        metadata: { source: "schedule" },
      }),
      makeOutputEvent({
        id: "tool-agent-1",
        sequence: 2,
        sessionId,
        ts: "2026-03-07T21:20:06Z",
        text: "I’m reading the entry points first, then I’ll summarize the key edges.",
        metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
      }),
      makeOutputEvent({
        id: "tool-agent-2",
        sequence: 3,
        sessionId,
        ts: "2026-03-07T21:20:10Z",
        toolCalls: [
          { id: "ta-list", name: "list_files", arguments: { path: "." } },
          {
            id: "ta-read",
            name: "read_file",
            arguments: { path: "/workspace/AGENTS.md" },
          },
          {
            id: "ta-search",
            name: "grep_files",
            arguments: { pattern: "**/auth*" },
          },
          {
            id: "ta-web",
            name: "search_web",
            arguments: { query: "daytona sandbox access issue" },
          },
          {
            id: "ta-bash",
            name: "daytona_exec",
            arguments: { command: "pwd && uname -s" },
          },
        ],
      }),
      makeInputEvent({
        id: "tool-user-2",
        sequence: 4,
        sessionId,
        text: "Now run the Daytona check and tell me what broke.",
        ts: "2026-03-07T21:21:00Z",
      }),
      {
        id: "tool-act-1",
        type: "act.started",
        ts: "2026-03-07T21:21:05Z",
        session_id: sessionId,
        context: { exec_id: "exec-daytona" },
        data: {
          headline: "Running Daytona sandbox checks",
          tool_calls: [
            {
              id: "tda-create",
              name: "daytona_create_sandbox",
              narration: "Created Daytona sandbox",
            },
            {
              id: "tda-run",
              name: "daytona_exec",
              narration: "Ran `pwd; uname -s`",
            },
            {
              id: "tda-write",
              name: "daytona_exec",
              narration: "Wrote `/home/daytona/hello-from-everruns.txt`",
            },
            {
              id: "tda-read",
              name: "daytona_exec",
              narration: "Read the file back",
            },
          ],
        },
      },
      {
        id: "tool-completed-1",
        type: "tool.completed",
        ts: "2026-03-07T21:21:07Z",
        session_id: sessionId,
        context: { exec_id: "exec-daytona" },
        data: {
          tool_call_id: "tda-create",
          tool_name: "daytona_create_sandbox",
          success: true,
          status: "success",
          narration: "Created Daytona sandbox",
          result: [
            {
              type: "text",
              text: JSON.stringify({
                sandbox_id: "9beac0d5-ae4c-41e8-970f-f48e70a20395",
                status: "running",
                workspace_path: "/home/daytona",
              }),
            },
          ],
          duration_ms: 620,
        },
      },
      {
        id: "tool-completed-2",
        type: "tool.completed",
        ts: "2026-03-07T21:21:08Z",
        session_id: sessionId,
        context: { exec_id: "exec-daytona" },
        data: {
          tool_call_id: "tda-run",
          tool_name: "daytona_exec",
          success: false,
          status: "error",
          narration: "Ran `pwd; uname -s`",
          error: "Resource was not created by this session",
          result: [{ type: "text", text: "Resource was not created by this session" }],
          duration_ms: 180,
        },
      },
      {
        id: "tool-completed-3",
        type: "tool.completed",
        ts: "2026-03-07T21:21:11Z",
        session_id: sessionId,
        context: { exec_id: "exec-daytona" },
        data: {
          tool_call_id: "tda-write",
          tool_name: "daytona_exec",
          success: true,
          status: "success",
          narration: "Wrote `/home/daytona/hello-from-everruns.txt`",
          result: [{ type: "text", text: "file written" }],
          duration_ms: 240,
        },
      },
      {
        id: "tool-completed-4",
        type: "tool.completed",
        ts: "2026-03-07T21:21:12Z",
        session_id: sessionId,
        context: { exec_id: "exec-daytona" },
        data: {
          tool_call_id: "tda-read",
          tool_name: "daytona_exec",
          success: true,
          status: "success",
          narration: "Read the file back",
          result: [{ type: "text", text: "hello from dev.everruns.com" }],
          duration_ms: 120,
        },
      },
      {
        id: "tool-act-1-done",
        type: "act.completed",
        ts: "2026-03-07T21:21:12Z",
        session_id: sessionId,
        context: { exec_id: "exec-daytona" },
        data: {
          completed: true,
          success_count: 3,
          error_count: 1,
          duration_ms: 1820,
          headline: "Ran Daytona sandbox checks",
        },
      },
      {
        id: "tool-client-wait",
        type: "tool.call_requested",
        ts: "2026-03-07T21:21:20Z",
        session_id: sessionId,
        context: {},
        data: {
          headline: "Waiting on tools",
          tool_calls: [
            {
              id: "client-pick",
              name: "pick_file",
              arguments: { accept: ".png,.jpg" },
            },
          ],
          tool_summaries: [
            {
              id: "client-pick",
              name: "pick_file",
              display_name: "Pick File",
              narration: "Choose a screenshot",
            },
          ],
        },
      },
      {
        // A URL mode elicitation: an MCP server needs a person to finish
        // something in their browser, so the turn holds on a consent card.
        id: "tool-url-elicitation",
        type: "tool.call_requested",
        ts: "2026-03-07T21:21:22Z",
        session_id: sessionId,
        context: {},
        data: {
          headline: "Waiting for the user",
          tool_calls: [
            {
              id: "url-elicitation-1",
              name: "confirm_url_elicitation",
              arguments: {
                server: "Example Billing",
                tool: "charge",
                retry_tool: "mcp_example_billing_charge",
                message: "Authorize the sandbox subscription to continue.",
                url: "https://billing.example.com/authorize/9f2c1b?session=demo",
                url_host: "billing.example.com",
                url_is_punycode: false,
              },
            },
          ],
          tool_summaries: [
            {
              id: "url-elicitation-1",
              name: "confirm_url_elicitation",
              display_name: "Confirm URL",
              narration: "Waiting for browser authorization",
            },
          ],
        },
      },
      makeOutputEvent({
        id: "tool-agent-3",
        sequence: 5,
        sessionId,
        ts: "2026-03-07T21:21:25Z",
        text: "The Daytona run succeeded after the first session-scoped command failed on resource ownership.",
        metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
      }),
    ];

    return {
      sessionId,
      events,
      toolResultsMap: new Map([
        [
          "ta-list",
          toolResult(
            "ta-list",
            "list_files",
            [{ type: "text", text: "apps/\ncrates/\nscripts/\nAGENTS.md" }],
            { duration_ms: 54 },
          ),
        ],
        [
          "ta-read",
          toolResult(
            "ta-read",
            "read_file",
            [
              {
                type: "text",
                text: JSON.stringify({
                  path: "/workspace/AGENTS.md",
                  content: "Always make sure you are working on top of latest main from remote.",
                  encoding: "text",
                  size_bytes: 78,
                }),
              },
            ],
            { duration_ms: 36 },
          ),
        ],
        [
          "ta-search",
          toolResult(
            "ta-search",
            "grep_files",
            [
              {
                type: "text",
                text: "crates/server/src/auth/config.rs\napps/ui/src/providers/auth-provider.tsx",
              },
            ],
            { duration_ms: 44 },
          ),
        ],
        [
          "ta-web",
          toolResult(
            "ta-web",
            "search_web",
            [
              {
                type: "text",
                text: "Auth bootstrap and middleware ordering can cause login redirects in dev.",
              },
            ],
            { duration_ms: 260 },
          ),
        ],
      ]),
      toolProgressMap: new Map([
        [
          "ta-web",
          {
            tool_call_id: "ta-web",
            tool_name: "search_web",
            message: "Comparing docs and UI behavior",
          },
        ],
      ]),
      toolOutputMap: new Map([["ta-bash", { stdout: "/home/daytona\nLinux\n", stderr: "" }]]),
      pendingImages: makePendingImages(),
      commands: devCommands,
      models: devModels,
      initialInputValue: "/ship",
      initialModelId: "model-kimi-k2.5",
      initialReasoningEffort: "medium",
    };
  }

  const sessionId = "session-dev-chat-components";
  const foldedTurnId = "turn-components-folded";
  const events: Event[] = [
    makeInputEvent({
      id: "components-user-1",
      sequence: 1,
      sessionId,
      text: "Tighten the tool-call UI and show me the exact surfaces users will see.",
      ts: "2026-03-07T21:45:00Z",
    }),
    makeOutputEvent({
      id: "components-agent-1",
      sequence: 2,
      sessionId,
      ts: "2026-03-07T21:45:04Z",
      text: "I’m rendering the runtime chat stack directly so the preview matches production behavior.",
      metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
    }),
    makeOutputEvent({
      id: "components-agent-2",
      sequence: 3,
      sessionId,
      ts: "2026-03-07T21:45:09Z",
      text: "Here’s the current pass over transcript chrome, todo state, and file previews.",
      toolCalls: [
        {
          id: "cc-todos",
          name: "write_todos",
          arguments: {
            todos: [
              {
                content: "Audit current transcript components",
                activeForm: "Auditing current transcript components",
                status: "completed",
              },
              {
                content: "Move dev routes onto real chat stack",
                activeForm: "Moving dev routes onto real chat stack",
                status: "in_progress",
              },
              {
                content: "Verify the preview against main chat",
                activeForm: "Verifying the preview against main chat",
                status: "pending",
              },
            ],
          },
        },
        {
          id: "cc-read-image",
          name: "read_file",
          arguments: { path: "/workspace/mockups/tool-call-flow.png" },
        },
        {
          id: "cc-write",
          name: "write_file",
          arguments: {
            path: "/workspace/apps/ui/src/components/chat/tool-call-card.tsx",
          },
        },
      ],
      metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
    }),
    {
      id: "components-tools-pending",
      type: "tool.call_requested",
      ts: "2026-03-07T21:45:13Z",
      session_id: sessionId,
      context: { turn_id: foldedTurnId },
      data: {
        headline: "Waiting on tools",
        tool_calls: [
          {
            id: "cc-client-pick",
            name: "pick_file",
            arguments: { accept: ".png,.jpg" },
          },
        ],
        tool_summaries: [
          {
            id: "cc-client-pick",
            name: "pick_file",
            display_name: "Pick File",
            narration: "Choose a screenshot to annotate",
          },
        ],
      },
    },
    {
      id: "components-act",
      type: "act.started",
      ts: "2026-03-07T21:45:18Z",
      session_id: sessionId,
      context: { turn_id: foldedTurnId, exec_id: "exec-components" },
      data: {
        headline: "Refining transcript chrome",
        tool_calls: [
          { id: "cc-shell", name: "bash", narration: "Ran UI tests" },
          {
            id: "cc-edit",
            name: "edit_file",
            narration: "Patched chat card styles",
          },
        ],
      },
    },
    {
      id: "components-shell-done",
      type: "tool.completed",
      ts: "2026-03-07T21:45:20Z",
      session_id: sessionId,
      context: { turn_id: foldedTurnId, exec_id: "exec-components" },
      data: {
        tool_call_id: "cc-shell",
        tool_name: "bash",
        success: true,
        status: "success",
        narration: "Ran UI tests",
        result: [
          {
            type: "text",
            text: JSON.stringify({
              stdout: "PASS src/__tests__/tool-activity-group.test.tsx",
              stderr: "",
              exit_code: 0,
              success: true,
            }),
          },
        ],
        duration_ms: 410,
      },
    },
    {
      id: "components-edit-done",
      type: "tool.completed",
      ts: "2026-03-07T21:45:22Z",
      session_id: sessionId,
      context: { turn_id: foldedTurnId, exec_id: "exec-components" },
      data: {
        tool_call_id: "cc-edit",
        tool_name: "edit_file",
        success: true,
        status: "success",
        narration: "Patched chat card styles",
        result: [{ type: "text", text: "diff applied" }],
        duration_ms: 130,
      },
    },
    {
      id: "components-act-done",
      type: "act.completed",
      ts: "2026-03-07T21:45:22Z",
      session_id: sessionId,
      context: { turn_id: foldedTurnId, exec_id: "exec-components" },
      data: {
        completed: true,
        success_count: 2,
        error_count: 0,
        duration_ms: 640,
        headline: "Refined transcript chrome",
      },
    },
    makeOutputEvent({
      id: "components-agent-3",
      sequence: 4,
      sessionId,
      ts: "2026-03-07T21:45:26Z",
      text: "The preview now uses the same transcript list and composer components as the live session view.",
      metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
      context: { turn_id: foldedTurnId },
    }),
    {
      id: "components-turn-done",
      type: "turn.completed",
      ts: "2026-03-07T21:45:27Z",
      session_id: sessionId,
      context: { turn_id: foldedTurnId },
      data: {
        turn_id: foldedTurnId,
        duration_ms: 9000,
      },
    },
  ];

  return {
    sessionId,
    events,
    toolResultsMap: new Map([
      [
        "cc-todos",
        toolResult("cc-todos", "write_todos", [{ type: "text", text: "todo list updated" }], {
          duration_ms: 28,
        }),
      ],
      [
        "cc-read-image",
        toolResult(
          "cc-read-image",
          "read_file",
          [
            {
              type: "text",
              text: JSON.stringify({
                path: "/workspace/mockups/tool-call-flow.png",
                media_type: "image/png",
                size_bytes: 68,
              }),
            },
            {
              type: "image",
              base64:
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO2pQwAAAABJRU5ErkJggg==",
              media_type: "image/png",
            },
          ],
          { duration_ms: 74 },
        ),
      ],
      [
        "cc-write",
        toolResult(
          "cc-write",
          "write_file",
          [
            {
              type: "text",
              text: JSON.stringify({
                path: "/workspace/apps/ui/src/components/chat/tool-call-card.tsx",
                size_bytes: 2134,
                created: false,
                applied_edits: 3,
                first_changed_line: 24,
              }),
            },
          ],
          { duration_ms: 92 },
        ),
      ],
    ]),
    toolProgressMap: new Map(),
    toolOutputMap: new Map(),
    pendingImages: makePendingImages(),
    commands: devCommands,
    models: devModels,
    initialInputValue: "/process-issues",
    initialModelId: "model-kimi-k2.5",
    initialReasoningEffort: "medium",
  };
}
