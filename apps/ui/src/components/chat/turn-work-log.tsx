"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";

interface TurnWorkLogProps {
  label: string;
  isActive: boolean;
  children: ReactNode;
}

export function TurnWorkLog({ label, isActive, children }: TurnWorkLogProps) {
  const [expanded, setExpanded] = useState(isActive);
  const wasActiveRef = useRef(isActive);

  useEffect(() => {
    if (isActive) {
      setExpanded(true);
    } else if (wasActiveRef.current) {
      setExpanded(false);
    }
    wasActiveRef.current = isActive;
  }, [isActive]);

  return (
    <div className="space-y-2">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
        className="group flex w-full items-center gap-2 pt-2 text-left text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <span className="flex min-h-5 min-w-5 items-center justify-center">
          {isActive ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : expanded ? (
            <ChevronDown className="h-4 w-4" />
          ) : (
            <ChevronRight className="h-4 w-4" />
          )}
        </span>
        <span className="whitespace-nowrap font-medium">{label}</span>
        <span className="h-px flex-1 bg-border transition-colors group-hover:bg-border/80" />
      </button>

      <div
        aria-hidden={!expanded}
        className={cn(
          "grid transition-all duration-200 ease-out",
          expanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="ml-7 space-y-3 pb-1">{children}</div>
        </div>
      </div>
    </div>
  );
}
