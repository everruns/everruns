"use client";

import { ChevronRight } from "lucide-react";
import type { ReactNode } from "react";

// The "More" list shared by the agent and harness config columns. Each row
// shows the current value and opens a side sheet.

export interface MoreSettingsRow {
  id: string;
  label: string;
  summary: string;
}

export function MoreSettingsNav({
  rows,
  onOpen,
  renderRow,
}: {
  rows: MoreSettingsRow[];
  onOpen: (id: string) => void;
  renderRow?: (row: MoreSettingsRow, content: ReactNode) => ReactNode;
}) {
  return (
    <nav aria-label="More settings" className="flex flex-col pt-2 pb-4">
      <p className="px-4 pt-2 pb-1 text-xs font-medium text-muted-foreground">More</p>
      {rows.map((row) => {
        const content = (
          <button
            key={row.id}
            type="button"
            onClick={() => onOpen(row.id)}
            className="flex w-full items-center gap-2 px-4 py-2 text-left text-[13px] transition-colors hover:bg-muted"
          >
            <span className="flex-1">{row.label}</span>
            <span className="min-w-0 truncate text-muted-foreground">{row.summary}</span>
            <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
          </button>
        );
        return renderRow ? renderRow(row, content) : content;
      })}
    </nav>
  );
}
