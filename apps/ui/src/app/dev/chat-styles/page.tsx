"use client";

import Link from "next/link";
import { ArrowLeft, Bot, Brain, CalendarClock, ImagePlus, Send, StopCircle } from "lucide-react";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import type { Event, ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";

const isDev = process.env.NODE_ENV === "development";

const userEvent: Event = {
  id: "evt-chat-style-user",
  type: "input.message",
  ts: "2026-03-07T21:40:00Z",
  session_id: "session-chat-style",
  context: {},
  data: {
    message: {
      id: "msg-chat-style-user",
      session_id: "session-chat-style",
      sequence: 1,
      role: "user",
      content: [{ type: "text", text: "Inspect this repo and summarize the rewrite plan." }],
      tool_call_id: null,
      created_at: "2026-03-07T21:40:00Z",
      metadata: { source: "schedule" },
    },
  },
};

const agentEvent: Event = {
  id: "evt-chat-style-agent",
  type: "output.message.completed",
  ts: "2026-03-07T21:40:05Z",
  session_id: "session-chat-style",
  context: {},
  data: {
    message: {
      id: "msg-chat-style-agent",
      session_id: "session-chat-style",
      sequence: 2,
      role: "agent",
      content: [
        {
          type: "text",
          text: "I'm checking the structure first, then I'll outline the smallest safe rewrite path.",
        },
      ],
      tool_call_id: null,
      created_at: "2026-03-07T21:40:05Z",
      metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
    },
    metadata: { model: "kimi-k2.5" },
    usage: { input_tokens: 954, output_tokens: 126 },
  },
};

const toolCalls: ToolCallContent[] = [
  { id: "style-list", name: "list_files", arguments: { path: "." } },
  { id: "style-read", name: "read_file", arguments: { path: "/workspace/README.md" } },
  { id: "style-search", name: "grep_files", arguments: { pattern: "**/package.json" } },
];

const toolResults = new Map<string, ToolCompletedData>([
  [
    "style-list",
    {
      tool_call_id: "style-list",
      tool_name: "list_files",
      success: true,
      status: "success",
      result: [{ type: "text", text: "README.md\napps/\ncrates/\njustfile" }],
    },
  ],
]);

function Note({ children }: { children: React.ReactNode }) {
  return (
    <div className="border border-border bg-card px-4 py-3 text-sm text-muted-foreground">
      {children}
    </div>
  );
}

export default function ChatStylesPage() {
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
          <p className="text-[11px] uppercase tracking-[0.3em] text-muted-foreground">Chat Style</p>
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            Canonical chat surface
          </h1>
          <p className="text-sm leading-6 text-muted-foreground">
            The application now uses this single style: sharp surfaces, left and right border
            accents, muted chrome, and info affordances on message rows.
          </p>
        </div>

        <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_320px]">
          <div className="overflow-hidden border border-border bg-card">
            <div className="border-b border-border px-5 py-4">
              <div className="text-[10px] uppercase tracking-[0.24em] text-muted-foreground">
                Live transcript pattern
              </div>
              <div className="mt-1 text-sm font-medium text-foreground">
                Default chat composition
              </div>
            </div>

            <div className="space-y-6 bg-background/80 px-4 py-5 sm:px-6">
              <div className="flex justify-end">
                <div className="max-w-[78%] border-r-2 border-r-accent bg-[hsl(var(--accent)/0.1)] px-4 py-3 text-sm text-foreground">
                  <div className="mb-1 flex items-center justify-between gap-2">
                    <div className="flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                      <CalendarClock className="h-3 w-3" />
                      Scheduled
                    </div>
                    <MessageInfoIcon event={userEvent} />
                  </div>
                  Inspect this repo and summarize the rewrite plan.
                </div>
              </div>

              <div className="flex items-start gap-3 pr-2">
                <div className="mt-1 flex h-6 w-6 flex-shrink-0 items-center justify-center border border-border bg-primary text-primary-foreground">
                  <Bot className="h-3.5 w-3.5" />
                </div>
                <div className="flex flex-1 items-start gap-2">
                  <div className="flex-1 border-l-2 border-l-primary bg-card px-4 py-3 text-sm leading-7 text-foreground">
                    I&apos;m checking the structure first, then I&apos;ll outline the smallest safe
                    rewrite path.
                  </div>
                  <MessageInfoIcon event={agentEvent} />
                </div>
              </div>

              <div className="ml-9">
                <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResults} />
              </div>
            </div>

            <div className="border-t border-border bg-muted/30 p-4">
              <div className="border border-border bg-background px-4 py-4 text-sm text-foreground">
                Type a message or <span className="font-mono">/</span> for commands...
              </div>
              <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
                <div className="flex flex-wrap items-center gap-3">
                  <button
                    type="button"
                    className="inline-flex h-10 w-10 items-center justify-center border border-border bg-background"
                    aria-label="Attach images"
                  >
                    <ImagePlus className="icon-sharp h-4 w-4" />
                  </button>
                  <div className="flex h-10 items-center gap-2 border border-border bg-background px-3 text-sm">
                    <span className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                      Model
                    </span>
                    <span className="text-foreground">Kimi K2.5</span>
                  </div>
                  <div className="flex h-10 items-center gap-2 border border-border bg-background px-3 text-sm">
                    <Brain className="icon-sharp h-3.5 w-3.5 text-muted-foreground" />
                    <span className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                      Reasoning
                    </span>
                    <span className="text-foreground">Default</span>
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

          <div className="space-y-4">
            <Note>User messages anchor on a right gold border, not a rounded bubble.</Note>
            <Note>
              Agent messages stay flatter: icon rail, left primary border, metadata icon on the
              side.
            </Note>
            <Note>
              Tool execution and todos inherit the same surface rules, so chat reads as one system.
            </Note>
          </div>
        </div>
      </div>
    </div>
  );
}
