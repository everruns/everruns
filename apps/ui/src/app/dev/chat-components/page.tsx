"use client";

import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ImageAttachments } from "@/components/chat/image-attachments";
import type { Event, ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";
import type { PendingImage } from "@/lib/api/images";

const isDev = process.env.NODE_ENV === "development";

const infoEvent: Event = {
  id: "evt-chat-components",
  type: "output.message.completed",
  ts: "2026-03-07T21:45:00Z",
  session_id: "session-chat-components",
  context: {},
  data: {
    message: {
      id: "msg-chat-components",
      session_id: "session-chat-components",
      sequence: 1,
      role: "agent",
      content: [{ type: "text", text: "Components now follow the canonical chat style." }],
      tool_call_id: null,
      created_at: "2026-03-07T21:45:00Z",
      metadata: { model: "kimi-k2.5", reasoning_effort: "medium" },
    },
    metadata: { model: "kimi-k2.5" },
    usage: { input_tokens: 318, output_tokens: 52 },
  },
};

const toolCalls: ToolCallContent[] = [
  { id: "component-list", name: "list_files", arguments: { path: "." } },
  { id: "component-read", name: "read_file", arguments: { path: "/workspace/AGENTS.md" } },
  { id: "component-search", name: "search_web", arguments: { query: "chat ui reference" } },
];

const toolResults = new Map<string, ToolCompletedData>([
  [
    "component-list",
    {
      tool_call_id: "component-list",
      tool_name: "list_files",
      success: true,
      status: "success",
      result: [{ type: "text", text: "apps/\ncrates/\nscripts/\nAGENTS.md" }],
    },
  ],
  [
    "component-read",
    {
      tool_call_id: "component-read",
      tool_name: "read_file",
      success: true,
      status: "success",
      result: [{ type: "text", text: "Use a port prefix per worktree/session." }],
    },
  ],
]);

const pendingImages: PendingImage[] = [
  {
    tempId: "pending-1",
    file: null,
    uploadPromise: null,
    imageId: "img-uploaded-1",
    filename: "layout.png",
    previewUrl:
      "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23ece7db' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%230a1636' font-size='10'%3EPNG%3C/text%3E%3C/svg%3E",
    status: "uploaded",
  },
  {
    tempId: "pending-2",
    file: null,
    uploadPromise: null,
    imageId: null,
    filename: "annotated.jpg",
    previewUrl:
      "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='80' viewBox='0 0 80 80'%3E%3Crect fill='%23f6e7c1' width='80' height='80'/%3E%3Ctext x='50%25' y='50%25' dominant-baseline='middle' text-anchor='middle' fill='%23d4a43a' font-size='10'%3EJPG%3C/text%3E%3C/svg%3E",
    status: "uploading",
  },
];

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="border border-border bg-card">
      <div className="border-b border-border px-4 py-4">
        <div className="text-[10px] uppercase tracking-[0.24em] text-muted-foreground">{title}</div>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>
      <div className="p-4">{children}</div>
    </section>
  );
}

export default function DevChatComponentsPage() {
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
            Chat Components
          </p>
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            Canonical component gallery
          </h1>
          <p className="text-sm leading-6 text-muted-foreground">
            These are the chat-specific primitives in the style now used by the main application.
          </p>
        </div>

        <div className="grid gap-6 lg:grid-cols-2">
          <Section
            title="Message Info"
            description="Metadata affordance for user and assistant messages."
          >
            <div className="flex items-center justify-between border border-border bg-background px-4 py-3">
              <div className="text-sm text-foreground">
                Components now follow the canonical chat style.
              </div>
              <MessageInfoIcon event={infoEvent} />
            </div>
          </Section>

          <Section
            title="Attachments"
            description="Pending image attachments now use the same sharp surface treatment."
          >
            <ImageAttachments images={pendingImages} onRemove={() => undefined} />
          </Section>

          <Section
            title="Tool Activity"
            description="Grouped execution cards used inline in transcripts."
          >
            <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResults} />
          </Section>

          <Section
            title="Execution Plan"
            description="Persistent todo card for long-running turns."
          >
            <TodoListRenderer
              arguments={{
                todos: [
                  {
                    content: "Read current implementation",
                    activeForm: "Reading current implementation",
                    status: "completed",
                  },
                  {
                    content: "Apply canonical styling",
                    activeForm: "Applying canonical styling",
                    status: "in_progress",
                  },
                  {
                    content: "Verify main routes",
                    activeForm: "Verifying main routes",
                    status: "pending",
                  },
                ],
              }}
              isExecuting
            />
          </Section>
        </div>
      </div>
    </div>
  );
}
