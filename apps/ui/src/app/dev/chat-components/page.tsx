"use client";

import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ImageAttachments } from "@/components/chat/image-attachments";
import type { Event, ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "@/components/chat/tool-call-utils";
import type { PendingImage } from "@/lib/api/images";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";

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
      content: [
        {
          type: "text",
          text: "Components now follow the canonical chat style.",
        },
      ],
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
  {
    id: "component-read",
    name: "read_file",
    arguments: { path: "/workspace/AGENTS.md" },
  },
  {
    id: "component-search",
    name: "search_web",
    arguments: { query: "chat ui reference" },
  },
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
    // Dev fixtures stay self-contained: no backend image record exists here.
    imageId: null,
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
  return (
    <DevPageShell
      eyebrow="Chat Components"
      title="Canonical component gallery"
      description="These are the chat-specific primitives used by the runtime chat surface."
    >
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
          description="Pending image attachments now use the same runtime treatment."
        >
          <ImageAttachments images={pendingImages} onRemove={() => undefined} />
        </Section>

        <Section
          title="Tool Activity"
          description="Grouped execution cards used inline in transcripts."
        >
          <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResults} />
        </Section>

        <Section title="Execution Plan" description="Persistent todo card for long-running turns.">
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
    </DevPageShell>
  );
}
