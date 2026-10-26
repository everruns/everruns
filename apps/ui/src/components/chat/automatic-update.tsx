/**
 * Decisions:
 * - A task wake is written for the coordinator. The transcript shows one line,
 *   the headline a person can read, and keeps the report folded. It is not
 *   something the person typed, so it does not take the user-turn wash.
 * - A wake with no report is the line only. There is nothing to open.
 * - An opened report scrolls inside the row, so a long proposal does not take
 *   over the transcript.
 */
"use client";

import { useId, useState, type ReactNode } from "react";
import { ChevronRight, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";

export function AutomaticUpdate({
  label,
  headline,
  body,
  trailing,
  defaultOpen = false,
}: {
  label: string;
  /** Readable status, such as "Releases is done". Omitted when the wake did not parse. */
  headline?: string;
  /** The report, question, or raw wake text. Blank means the line does not open. */
  body?: string;
  trailing?: ReactNode;
  defaultOpen?: boolean;
}) {
  const report = body?.trim() ?? "";
  const canOpen = report.length > 0;
  const [open, setOpen] = useState(defaultOpen && canOpen);
  const bodyId = useId();
  const title = headline?.trim() ?? "";

  const row = (
    <>
      <span className="flex min-h-5 min-w-5 shrink-0 items-center justify-center">
        {canOpen ? (
          <ChevronRight
            className={cn(
              "h-4 w-4 transition-transform duration-200 ease-out motion-reduce:transition-none",
              open && "rotate-90",
            )}
          />
        ) : (
          <span className="h-4 w-4" />
        )}
      </span>
      <RefreshCw className="h-3.5 w-3.5 shrink-0" aria-hidden />
      <span className="shrink-0 font-medium">{label}</span>
      {title ? <span className="min-w-0 truncate text-foreground">{title}</span> : null}
      <span className="h-px min-w-4 flex-1 bg-border" />
    </>
  );

  return (
    <div className="animate-chat-row-in" data-testid="automatic-update">
      <div className="flex items-center gap-1">
        {canOpen ? (
          <button
            type="button"
            aria-expanded={open}
            aria-controls={bodyId}
            onClick={() => setOpen((value) => !value)}
            className="group flex min-w-0 flex-1 items-center gap-2 py-1 text-left text-sm text-muted-foreground transition-colors hover:text-foreground"
          >
            {row}
          </button>
        ) : (
          <div className="flex min-w-0 flex-1 items-center gap-2 py-1 text-sm text-muted-foreground">
            {row}
          </div>
        )}
        {trailing ? <div className="shrink-0">{trailing}</div> : null}
      </div>
      {canOpen && open ? (
        <p
          id={bodyId}
          className="ml-7 max-h-48 overflow-y-auto whitespace-pre-wrap pr-2 text-sm leading-[1.6] text-muted-foreground"
        >
          {report}
        </p>
      ) : null}
    </div>
  );
}
