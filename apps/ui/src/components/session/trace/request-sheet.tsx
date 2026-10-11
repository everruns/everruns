"use client";

// Full request of one model call, opened from the inspector's Request section.
// Two views of the same call: Messages (one row per message, new ones marked,
// the estimated end of the cached prefix drawn in, each row expandable to its
// rendered content) and JSON (the request as stored, whole).
//
// Decision: roles show what the provider driver receives (`assistant`,
// `tool`), not the stored runtime roles (`agent`, `tool_result`). The exact
// wire body is not recorded (TM-LLM-039), so JSON is the stored request and
// says so.

import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { ContentPartRenderer } from "@/components/llm/llm-history-viewer";
import { useTraceRequest } from "@/hooks/use-session-trace";
import type { ContentPart, TraceRequestMessage, TraceRequestPage } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { CacheBar } from "./cache-bar";
import { JsonView, formatBytes } from "./json-view";
import { formatCount } from "./trace-model";

type View = "messages" | "json";

export const ROLE_COLOR: Record<string, string> = {
  system: "text-muted-foreground",
  user: "text-accent-foreground",
  assistant: "text-primary dark:text-accent",
  tool: "text-foreground",
};

export const ROLE_HINT =
  "Role as handed to the provider driver. Stored messages say agent and tool_result; each driver maps these onto its own wire format.";

export function RequestSheet({
  sessionId,
  turn,
  step,
  open,
  onOpenChange,
}: {
  sessionId: string;
  turn: number;
  step: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [view, setView] = useState<View>("messages");
  const selection = open ? { turn, step } : null;
  const messagesQuery = useTraceRequest(sessionId, view === "messages" ? selection : null);
  const jsonQuery = useTraceRequest(sessionId, view === "json" ? selection : null, true);
  const query = view === "json" ? jsonQuery : messagesQuery;
  const pages = query.data?.pages ?? [];
  const first = pages[0];
  const messages = pages.flatMap((page) => page.messages);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-3 sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Model request</DialogTitle>
          <DialogDescription>
            Step #{step} · Turn {formatCount(turn)}
            {first &&
              ` · ${formatCount(first.message_count)} messages${first.model ? ` · ${first.model}` : ""}${first.provider ? ` · ${first.provider}` : ""}`}
          </DialogDescription>
        </DialogHeader>
        {first?.usage && <CacheBar usage={first.usage} />}
        <div className="flex items-center gap-2">
          <div className="inline-flex border text-xs" role="group" aria-label="Request view">
            {(["messages", "json"] as const).map((v) => (
              <button
                key={v}
                type="button"
                aria-pressed={view === v}
                onClick={() => setView(v)}
                className={cn(
                  "px-2.5 py-1 font-medium",
                  view === v ? "bg-foreground text-background" : "hover:bg-muted",
                )}
              >
                {v === "messages" ? "Messages" : "JSON"}
              </button>
            ))}
          </div>
          {view === "json" && first && (
            <CopyButton value={JSON.stringify(storedRequest(pages), null, 2)} label="Copy JSON" />
          )}
          <span className="ml-auto text-[11px] text-muted-foreground">
            {view === "json"
              ? "As stored; the driver reshapes it for the provider"
              : "Roles as the provider driver receives them"}
          </span>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto border">
          {query.isLoading ? (
            <div className="space-y-2 p-3">
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-2/3" />
            </div>
          ) : query.error ? (
            <div className="p-3 text-sm text-destructive">Could not load the request.</div>
          ) : view === "json" ? (
            <JsonView value={storedRequest(pages)} className="max-h-none border-0 bg-transparent" />
          ) : (
            <ul className="text-xs">
              {first && first.tools.length > 0 && <ToolsRow tools={first.tools} />}
              {first?.usage?.cached_through === -1 && (
                <CacheBoundary tokens={first.usage.cache_read_tokens} />
              )}
              {messages.map((message) => (
                <MessageRowWithBoundary
                  key={message.index}
                  message={message}
                  first={first}
                  isNew={!!first && message.index >= first.new_from}
                />
              ))}
            </ul>
          )}
        </div>
        {query.hasNextPage && (
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            onClick={() => void query.fetchNextPage()}
            disabled={query.isFetchingNextPage}
          >
            Load more messages ({formatCount(messages.length)} of{" "}
            {formatCount(first?.message_count ?? 0)})
          </Button>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** The loaded pages as one request object, in the shape `llm.generation` stores. */
function storedRequest(pages: TraceRequestPage[]) {
  const first = pages[0];
  return {
    model: first?.model ?? null,
    provider: first?.provider ?? null,
    request_options: first?.request_options ?? null,
    tools: first?.tools ?? [],
    messages: pages.flatMap((page) => page.messages.map((m) => m.content ?? m.preview)),
  };
}

function MessageRowWithBoundary({
  message,
  first,
  isNew,
}: {
  message: TraceRequestMessage;
  first: TraceRequestPage | undefined;
  isNew: boolean;
}) {
  const cachedThrough = first?.usage?.cached_through;
  const cached = cachedThrough != null && message.index <= cachedThrough;
  return (
    <>
      <MessageRow message={message} isNew={isNew} cached={cached} />
      {cachedThrough != null && message.index === cachedThrough && first?.usage && (
        <CacheBoundary tokens={first.usage.cache_read_tokens} />
      )}
    </>
  );
}

function CacheBoundary({ tokens }: { tokens: number }) {
  return (
    <li
      className="flex items-center gap-2 border-t border-dashed border-success/60 bg-success/6 px-2.5 py-1 text-[11px] text-success"
      title="Providers report how many tokens came from cache, not where the cache ended. The line is that share laid over the request by size."
    >
      <span className="font-medium">≈ cached prefix ends here</span>
      <span className="text-muted-foreground">
        {formatCount(tokens)} tokens read from cache (estimated position)
      </span>
    </li>
  );
}

function ToolsRow({ tools }: { tools: TraceRequestPage["tools"] }) {
  const [open, setOpen] = useState(false);
  return (
    <li className="border-b border-border/60">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left hover:bg-muted/60"
      >
        {open ? (
          <ChevronDown className="size-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="size-3 shrink-0 text-muted-foreground" />
        )}
        <span className="w-[62px] shrink-0 font-mono text-[11px] text-muted-foreground">tools</span>
        <span className="min-w-0 flex-1 truncate">
          {formatCount(tools.length)} tools: {tools.map((t) => t.name).join(", ")}
        </span>
      </button>
      {open && (
        <ul className="space-y-1.5 border-t border-border/60 px-2.5 py-2">
          {tools.map((tool) => (
            <li key={tool.name} className="flex gap-2">
              <span className="w-40 shrink-0 truncate font-mono text-[11px]">{tool.name}</span>
              <span className="min-w-0 flex-1 text-muted-foreground">{tool.description}</span>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

function MessageRow({
  message,
  isNew,
  cached,
}: {
  message: TraceRequestMessage;
  isNew: boolean;
  cached: boolean;
}) {
  const [open, setOpen] = useState(false);
  const parts = useMemo(() => contentParts(message.content), [message.content]);
  return (
    <li
      className={cn(
        "border-t border-l-2 border-t-border/60 border-l-transparent first:border-t-0",
        cached && "border-l-success/50",
        isNew && "bg-accent/6",
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left hover:bg-muted/60"
      >
        <span className="w-8 shrink-0 font-mono text-[11px] text-muted-foreground">
          {message.index}
        </span>
        <span
          className={cn(
            "w-[62px] shrink-0 font-mono text-[11px]",
            ROLE_COLOR[message.role] ?? "text-info",
          )}
          title={ROLE_HINT}
        >
          {message.role}
        </span>
        <span className={cn("min-w-0 flex-1", open ? "break-words" : "truncate")}>
          {message.preview || "—"}
        </span>
        <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
          {formatBytes(message.size_bytes)}
        </span>
        {isNew && (
          <span className="shrink-0 border border-accent bg-accent/15 px-1 text-[10px]">new</span>
        )}
      </button>
      {open && (
        <div className="space-y-2 border-t border-border/60 px-2.5 py-2">
          {parts ? (
            parts.map((part, i) => <ContentPartRenderer key={i} part={part} />)
          ) : message.content ? (
            <JsonView value={message.content} />
          ) : (
            <p className="text-muted-foreground">
              Over 50 KB, so not inlined here. The JSON view loads it whole.
            </p>
          )}
        </div>
      )}
    </li>
  );
}

/** Content parts of a stored message, when it has them. */
function contentParts(content: unknown): ContentPart[] | null {
  if (!content || typeof content !== "object") return null;
  const parts = (content as { content?: unknown }).content;
  return Array.isArray(parts) && parts.every((p) => p && typeof p === "object" && "type" in p)
    ? (parts as ContentPart[])
    : null;
}
