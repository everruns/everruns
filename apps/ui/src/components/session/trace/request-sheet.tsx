"use client";

// Full request of one model call, opened from the inspector's Request section.
// Three views of the same call: Messages (one row per message, new ones
// marked, the estimated end of the cached prefix drawn in, each row expandable
// to its rendered content), Stored JSON (the request as the runtime keeps it)
// and Native JSON (the body the driver sent the provider, when recorded).
//
// Decision: Messages roles show what the provider driver receives
// (`assistant`, `tool`), not the stored runtime roles (`agent`,
// `tool_result`); Native JSON shows the provider's own. Calls made before the
// body was recorded, or by drivers that do not capture it, have no Native view.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import { ChevronDown, ChevronRight, Download, FileText } from "lucide-react";
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
import { buildImageSrc } from "@/components/chat/tool-call-utils";
import { useTraceRequest, useTraceRequestMessage } from "@/hooks/use-session-trace";
import { getFileUrl } from "@/lib/api/files";
import { getImageUrl, getThumbnailUrl } from "@/lib/api/images";
import type { ContentPart, TraceRequestMessage, TraceRequestPage } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { CacheBar } from "./cache-bar";
import { JsonView, formatBytes } from "./json-view";
import { formatCount } from "./trace-model";

type View = "messages" | "stored" | "native";
export type RequestFocus = "tools" | number;
type Call = { sessionId: string; turn: number; step: number };

const VIEW_LABEL: Record<View, string> = {
  messages: "Messages",
  stored: "Stored JSON",
  native: "Native JSON",
};

const VIEW_NOTE: Record<View, string> = {
  messages: "Roles as the provider driver receives them",
  stored: "As the runtime stores it; the driver reshapes it for the provider",
  native: "Exactly as sent to the provider; inline media and reasoning signatures omitted",
};

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
  focus,
}: Call & {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  focus?: RequestFocus;
}) {
  const ui = useUiTranslations();
  const [view, setView] = useState<View>("messages");
  const selection = open ? { turn, step } : null;
  const focusedIndex = typeof focus === "number" ? focus : undefined;
  // The Messages query supplies stable metadata even while switching to a JSON view.
  const messagesQuery = useTraceRequest(sessionId, selection, "messages", focusedIndex);
  const storedQuery = useTraceRequest(sessionId, view === "stored" ? selection : null, "full");
  const nativeQuery = useTraceRequest(sessionId, view === "native" ? selection : null, "native");
  const query = view === "stored" ? storedQuery : view === "native" ? nativeQuery : messagesQuery;
  const pages = query.data?.pages ?? [];
  const first = messagesQuery.data?.pages[0];
  const hasNative = first?.has_native_request ?? false;
  const jsonValue =
    view === "stored" ? storedRequest(pages) : view === "native" ? pages[0]?.native_request : null;
  const messages = pages.flatMap((page) => page.messages);
  return (
    <Dialog
      scope="viewport"
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) setView("messages");
        onOpenChange(nextOpen);
      }}
    >
      <DialogContent className="flex h-[calc(100dvh-2rem)] max-h-[calc(100%-2rem)] flex-col gap-3 overflow-hidden p-3 sm:max-w-[calc(100%-2rem)] sm:p-6">
        <DialogHeader className="shrink-0 pr-8 text-left">
          <DialogTitle>{ui("Model request")}</DialogTitle>
          <DialogDescription>
            {ui("Step #")}
            {step} {ui("· Turn")} {formatCount(turn)}
            {first &&
              ` · ${formatCount(first.message_count)} messages${first.model ? ` · ${first.model}` : ""}${first.provider ? ` · ${first.provider}` : ""}`}
          </DialogDescription>
        </DialogHeader>
        {first?.usage && <CacheBar usage={first.usage} />}
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <div className="inline-flex border text-xs" role="group" aria-label={ui("Request view")}>
            {(["messages", "stored", "native"] as const).map((v) => {
              const unavailable = v === "native" && !hasNative;
              return (
                <button
                  key={v}
                  type="button"
                  aria-pressed={view === v}
                  disabled={unavailable}
                  title={
                    unavailable
                      ? ui(
                          "Not recorded for this call: an older call, or a driver that does not capture its request",
                        )
                      : undefined
                  }
                  onClick={() => setView(v)}
                  className={cn(
                    "border-l px-2.5 py-1 font-medium first:border-l-0 disabled:cursor-not-allowed disabled:text-muted-foreground/60",
                    view === v ? "bg-foreground text-background" : "enabled:hover:bg-muted",
                  )}
                >
                  {VIEW_LABEL[v]}
                </button>
              );
            })}
          </div>
          {jsonValue != null && (
            <CopyButton value={JSON.stringify(jsonValue, null, 2)} label={ui("Copy JSON")} />
          )}
          <span className="ml-auto text-right text-[11px] text-muted-foreground">
            {VIEW_NOTE[view]}
          </span>
        </div>
        <div
          key={view}
          role="region"
          aria-label={ui(VIEW_LABEL[view])}
          tabIndex={0}
          className="min-h-0 flex-1 overflow-auto overscroll-contain border"
        >
          {query.isLoading ? (
            <div className="space-y-2 p-3">
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-2/3" />
            </div>
          ) : query.error ? (
            <div className="p-3 text-sm text-destructive">{ui("Could not load the request.")}</div>
          ) : view !== "messages" ? (
            <JsonView
              value={jsonValue}
              className="max-h-none overflow-visible border-0 bg-transparent"
            />
          ) : (
            <ul className="text-xs">
              {messagesQuery.hasPreviousPage && (
                <li className="border-b px-2.5 py-1.5">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => void messagesQuery.fetchPreviousPage()}
                    disabled={messagesQuery.isFetching}
                  >
                    {ui("Load earlier messages")}
                  </Button>
                </li>
              )}
              {first && first.tools.length > 0 && (
                <ToolsRow tools={first.tools} initiallyOpen={focus === "tools"} />
              )}
              {!messagesQuery.hasPreviousPage && first?.usage?.cached_through === -1 && (
                <CacheBoundary tokens={first.usage.cache_read_tokens} />
              )}
              {messages.map((message) => (
                <MessageRowWithBoundary
                  key={message.index}
                  message={message}
                  first={first}
                  isNew={!!first && message.index >= first.new_from}
                  call={{ sessionId, turn, step }}
                  initiallyOpen={message.index === focusedIndex}
                />
              ))}
            </ul>
          )}
        </div>
        {query.hasNextPage && view !== "native" && (
          <Button
            variant="outline"
            size="sm"
            className="self-start"
            onClick={() => void query.fetchNextPage()}
            disabled={query.isFetching}
          >
            {ui("Load more messages (")}
            {formatCount(messages.length)} {ui("of")} {formatCount(first?.message_count ?? 0)})
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
  call,
  initiallyOpen,
}: {
  message: TraceRequestMessage;
  first: TraceRequestPage | undefined;
  isNew: boolean;
  call: Call;
  initiallyOpen: boolean;
}) {
  const cachedThrough = first?.usage?.cached_through;
  const cached = cachedThrough != null && message.index <= cachedThrough;
  return (
    <>
      <MessageRow
        message={message}
        isNew={isNew}
        cached={cached}
        call={call}
        initiallyOpen={initiallyOpen}
      />
      {cachedThrough != null && message.index === cachedThrough && first?.usage && (
        <CacheBoundary tokens={first.usage.cache_read_tokens} />
      )}
    </>
  );
}

function CacheBoundary({ tokens }: { tokens: number }) {
  const ui = useUiTranslations();
  return (
    <li
      className="flex items-center gap-2 border-t border-dashed border-success/60 bg-success/6 px-2.5 py-1 text-[11px] text-success"
      title={ui(
        "Providers report how many tokens came from cache, not where the cache ended. The line is that share laid over the request by size.",
      )}
    >
      <span className="font-medium">{ui("≈ cached prefix ends here")}</span>
      <span className="text-muted-foreground">
        {formatCount(tokens)} {ui("tokens read from cache (estimated position)")}
      </span>
    </li>
  );
}

function ToolsRow({
  tools,
  initiallyOpen,
}: {
  tools: TraceRequestPage["tools"];
  initiallyOpen: boolean;
}) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(initiallyOpen);
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
        <span className="w-[62px] shrink-0 font-mono text-[11px] text-muted-foreground">
          {ui("tools")}
        </span>
        <span className="min-w-0 flex-1 truncate">
          {formatCount(tools.length)} {open ? ui("tools") : ui("tools:")}
          {!open && ` ${tools.map((t) => t.name).join(", ")}`}
        </span>
      </button>
      {open && (
        <ul className="space-y-1.5 border-t border-border/60 px-2.5 py-2">
          {tools.map((tool) => (
            <li
              key={tool.name}
              className="grid gap-1 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)] sm:gap-3"
            >
              <span className="min-w-0 font-mono text-[11px] wrap-anywhere">{tool.name}</span>
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
  call,
  initiallyOpen,
}: {
  message: TraceRequestMessage;
  isNew: boolean;
  cached: boolean;
  call: Call;
  initiallyOpen: boolean;
}) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(initiallyOpen);
  const ExpandedChevron = open ? ChevronDown : ChevronRight;
  return (
    <li
      ref={initiallyOpen ? scrollToMessage : undefined}
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
        <ExpandedChevron className="size-3 shrink-0 text-muted-foreground" aria-hidden />
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
        <span className="min-w-0 flex-1 truncate">{!open && (message.preview || "—")}</span>
        <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
          {formatBytes(message.size_bytes)}
        </span>
        {isNew && (
          <span className="shrink-0 border border-accent bg-accent/15 px-1 text-[10px]">
            {ui("new")}
          </span>
        )}
      </button>
      {open && <MessageBody call={call} message={message} />}
    </li>
  );
}

function scrollToMessage(row: HTMLLIElement | null) {
  row?.scrollIntoView({ block: "start", inline: "nearest" });
}

function MessageBody({ call, message }: { call: Call; message: TraceRequestMessage }) {
  const ui = useUiTranslations();
  const [loadFull, setLoadFull] = useState(false);
  const full = useTraceRequestMessage(call.sessionId, call, message.index, loadFull);
  const content = message.content ?? full.data;
  const parts = contentParts(content);
  return (
    <div className="space-y-2 overflow-x-auto border-t border-border/60 px-3 py-3 wrap-anywhere">
      {parts ? (
        parts.map((part, i) => <RequestContentPart key={i} part={part} />)
      ) : content != null ? (
        <JsonView value={content} className="max-h-none overflow-visible" />
      ) : (
        <>
          {message.preview && <p className="text-sm whitespace-pre-wrap">{message.preview}</p>}
          {loadFull && !full.isFetching ? (
            <p className="text-muted-foreground">{ui("Full message is unavailable.")}</p>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setLoadFull(true)}
              disabled={loadFull}
            >
              {ui("Load full message")}
            </Button>
          )}
        </>
      )}
    </div>
  );
}

function RequestContentPart({ part }: { part: ContentPart }) {
  const ui = useUiTranslations();
  if (part.type === "file") {
    return (
      <a
        href={getFileUrl(part.file_id)}
        download
        className="inline-flex max-w-full items-center gap-2 border bg-muted/50 px-2.5 py-2 hover:bg-muted"
      >
        <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <span className="min-w-0">{part.filename || part.file_id}</span>
        <Download className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        <span className="sr-only">{ui("Download")}</span>
      </a>
    );
  }
  if (part.type === "image_file" || part.type === "image") {
    // A captured remote URL is request data. Only fetch from our image store or
    // display recorded raster bytes, so inspecting a trace never contacts a remote host.
    const imageId = part.type === "image_file" ? encodeURIComponent(part.image_id) : null;
    const src = imageId
      ? getThumbnailUrl(imageId)
      : part.type === "image"
        ? buildImageSrc({ base64: part.base64, media_type: part.media_type })
        : null;
    const name = (part.type === "image_file" && part.filename) || ui("Image");
    if (src) {
      const image = (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={src}
          alt={name}
          loading="lazy"
          className="max-h-64 max-w-full border object-contain"
        />
      );
      return imageId ? (
        <a
          href={getImageUrl(imageId)}
          target="_blank"
          rel="noreferrer"
          className="inline-block max-w-full"
          title={ui("Click to view full size")}
        >
          {image}
        </a>
      ) : (
        image
      );
    }
  }
  return <ContentPartRenderer part={part} />;
}

/** Content parts of a stored message, when it has them. */
function contentParts(content: unknown): ContentPart[] | null {
  if (!content || typeof content !== "object") return null;
  const parts = (content as { content?: unknown }).content;
  return Array.isArray(parts) && parts.every((p) => p && typeof p === "object" && "type" in p)
    ? (parts as ContentPart[])
    : null;
}
