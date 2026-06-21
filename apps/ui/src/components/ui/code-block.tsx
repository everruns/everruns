"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "./button";
import { cn } from "@/lib/utils";

export interface CodeBlockSample {
  /** Tab label shown when there is more than one sample. */
  label: string;
  /** Optional language hint (currently informational; no highlighting theme). */
  language?: string;
  code: string;
}

interface CodeBlockProps {
  samples: CodeBlockSample[];
  className?: string;
}

/**
 * Monospaced code block with optional language tabs and a copy button. The app
 * styles code as plain `<pre>`/`<code>` over `bg-muted`; this keeps that look
 * while adding multi-language switching and copy-to-clipboard in one place.
 */
export function CodeBlock({ samples, className }: CodeBlockProps) {
  const [active, setActive] = useState(0);
  const [copied, setCopied] = useState(false);
  const current = samples[active] ?? samples[0];

  if (!current) return null;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(current.code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard can reject in insecure contexts or when permission is denied;
      // swallow so the click never surfaces an unhandled rejection.
    }
  };

  return (
    <div className={cn("overflow-hidden rounded-md border bg-muted", className)}>
      <div className="flex items-center justify-between gap-2 border-b bg-muted/50 px-2 py-1">
        <div className="flex flex-wrap gap-0.5" role="group" aria-label="Code language">
          {samples.length > 1 &&
            samples.map((sample, index) => (
              <button
                key={`${index}-${sample.label}`}
                type="button"
                aria-pressed={index === active}
                onClick={() => setActive(index)}
                className={cn(
                  "px-2 py-1 text-xs font-medium transition-colors",
                  index === active
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {sample.label}
              </button>
            ))}
          {samples.length === 1 && (
            <span className="px-2 py-1 text-xs font-medium text-muted-foreground">
              {current.label}
            </span>
          )}
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6 shrink-0 p-0"
          onClick={handleCopy}
          title={copied ? "Copied!" : "Copy"}
        >
          {copied ? (
            <Check className="h-3.5 w-3.5 text-green-500" />
          ) : (
            <Copy className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </Button>
      </div>
      <pre className="overflow-x-auto p-3 text-xs leading-relaxed">
        <code>{current.code}</code>
      </pre>
    </div>
  );
}
