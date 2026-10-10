"use client";

// Full request of one model call: every message it was sent, paged, each
// expandable to its whole content. Opened from the inspector's Request section.

import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { useTraceRequest } from "@/hooks/use-session-trace";
import type { TraceRequestMessage } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { JsonView, formatBytes } from "./json-view";
import { formatCount } from "./trace-model";

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
  const query = useTraceRequest(sessionId, open ? { turn, step } : null);
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
              ` · ${formatCount(first.message_count)} messages${first.model ? ` · ${first.model}` : ""}`}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto border">
          {query.isLoading ? (
            <div className="space-y-2 p-3">
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-2/3" />
            </div>
          ) : query.error ? (
            <div className="p-3 text-sm text-destructive">Could not load the request.</div>
          ) : (
            <ul className="text-xs">
              {messages.map((message) => (
                <MessageRow
                  key={message.index}
                  message={message}
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

function MessageRow({ message, isNew }: { message: TraceRequestMessage; isNew: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <li className={cn("border-t border-border/60 first:border-t-0", isNew && "bg-accent/6")}>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left hover:bg-muted/60"
      >
        <span className="w-8 shrink-0 font-mono text-[11px] text-muted-foreground">
          {message.index}
        </span>
        <span className="w-[62px] shrink-0 font-mono text-[11px]">{message.role}</span>
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
        <div className="border-t border-border/60 px-2.5 py-2">
          <JsonView value={message.content ?? undefined} text={message.preview} />
        </div>
      )}
    </li>
  );
}
