"use client";

// Search box of the Trace tab: full-text search over the session's events,
// newest hits first. Picking a hit hands its sequence to the view, which loads
// the turn that holds it and selects the step.

import { useEffect, useRef, useState } from "react";
import { SearchInput } from "@/components/ui/search-input";
import { useTraceSearch } from "@/hooks/use-session-trace";
import { cn } from "@/lib/utils";
import { formatClock, searchSnippet } from "./trace-model";

export function TraceSearch({
  sessionId,
  onPick,
}: {
  sessionId: string;
  onPick: (sequence: number) => void;
}) {
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = setTimeout(() => setQuery(text), 250);
    return () => clearTimeout(timer);
  }, [text]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!boxRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, []);

  const search = useTraceSearch(sessionId, query);
  const hits = (search.data ?? []).filter((e) => e.sequence != null);
  const searching = query.trim().length >= 2;

  const pick = (sequence: number) => {
    setOpen(false);
    onPick(sequence);
  };

  return (
    <div ref={boxRef} className="relative w-full sm:w-64">
      <SearchInput
        value={text}
        placeholder="Search events"
        aria-label="Search events"
        className="h-7 text-xs"
        onChange={(e) => {
          setText(e.target.value);
          setActive(0);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setOpen(false);
            e.currentTarget.blur();
          } else if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive((i) => Math.min(hits.length - 1, i + 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive((i) => Math.max(0, i - 1));
          } else if (e.key === "Enter" && hits[active]?.sequence != null) {
            e.preventDefault();
            pick(hits[active].sequence!);
          }
        }}
      />
      {open && searching && (
        <div
          role="listbox"
          aria-label="Search results"
          className="absolute top-full right-0 z-30 mt-1 max-h-80 w-[min(28rem,calc(100vw-2rem))] overflow-y-auto border bg-popover text-xs shadow-md"
        >
          {search.isLoading ? (
            <div className="px-3 py-2 text-muted-foreground">Searching…</div>
          ) : search.error ? (
            <div className="px-3 py-2 text-destructive">Search failed.</div>
          ) : hits.length === 0 ? (
            <div className="px-3 py-2 text-muted-foreground">No events match.</div>
          ) : (
            hits.map((event, index) => (
              <button
                key={event.id}
                type="button"
                role="option"
                aria-selected={index === active}
                onMouseEnter={() => setActive(index)}
                onClick={() => pick(event.sequence!)}
                className={cn(
                  "block w-full border-b border-border/60 px-3 py-2 text-left last:border-b-0",
                  index === active && "bg-muted",
                )}
              >
                <div className="flex gap-2 font-mono text-[11px] text-muted-foreground">
                  <span className="text-foreground">{event.type}</span>
                  <span className="ml-auto">{formatClock(event.ts)}</span>
                  <span>#{event.sequence}</span>
                </div>
                <div className="mt-0.5 truncate">{searchSnippet(event.data, query)}</div>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
