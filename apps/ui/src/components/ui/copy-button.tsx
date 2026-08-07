"use client";

import { useEffect, useRef, useState } from "react";
import { Copy, Hash, Check } from "lucide-react";
import { Button } from "./button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "./tooltip";
import { cn } from "@/lib/utils";

interface CopyButtonProps {
  value: string;
  /** Accessible name and tooltip text. Entity IDs must include the full exact value. */
  label?: string;
  kind?: "value" | "id";
  className?: string;
}

export function CopyButton({ value, label = "Copy", kind = "value", className }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (resetTimer.current) clearTimeout(resetTimer.current);
    },
    [],
  );

  const handleCopy = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    await navigator.clipboard.writeText(value);
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), 1500);
  };

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className={cn("-my-1 shrink-0 p-0", className)}
            onClick={handleCopy}
            aria-label={label}
          >
            {copied ? (
              <Check className="h-3 w-3 text-green-500" />
            ) : kind === "id" ? (
              <Hash className="h-3 w-3 text-muted-foreground" />
            ) : (
              <Copy className="h-3 w-3 text-muted-foreground" />
            )}
            <span className="sr-only" role="status" aria-live="polite">
              {copied ? "Copied" : ""}
            </span>
          </Button>
        </TooltipTrigger>
        <TooltipContent className="max-w-[min(40rem,calc(100vw-2rem))] break-all font-mono">
          <span>{label}</span>
          {copied && <span className="ml-2 text-green-600">Copied</span>}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
