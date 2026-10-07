"use client";

// The instructions is the agent page: the wide left pane of the workspace.
// Viewing renders the markdown at reading size; editing turns the same pane
// into a mono textarea in the same place, so moving between the two never
// changes the layout. One header row, no second row of tabs: "Source" is a
// ghost sm toggle for the raw text, sized with Edit prompt.

import { useEffect, useRef, useState } from "react";
import { Pencil } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { StreamdownMessage } from "@/components/chat/streamdown-message";
import { formatTokens, pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";

/** Rough token estimate (≈4 characters per token); labelled as approximate. */
export function promptStats(prompt: string): { words: number; tokens: number } {
  const trimmed = prompt.trim();
  return {
    words: trimmed ? trimmed.split(/\s+/).length : 0,
    tokens: Math.ceil(prompt.length / 4),
  };
}

interface AgentPromptPaneProps {
  value: string;
  editing: boolean;
  /** Absent for read-only agents: the pane stays a reader. */
  onEdit?: () => void;
  onChange?: (value: string) => void;
  /** Untrusted imported instructions must not render raw HTML. */
  skipHtml?: boolean;
  error?: string;
  /** Rendered under the editor in edit mode: checks about the prompt sit beside it. */
  checks?: React.ReactNode;
  className?: string;
  /** Shown when the prompt is empty. A harness uses this to say it contributes none. */
  emptyMessage?: string;
  placeholder?: string;
  /** Shorter editor, for a side sheet rather than the page. */
  compact?: boolean;
}

export function AgentPromptPane({
  value,
  editing,
  onEdit,
  onChange,
  skipHtml,
  error,
  checks,
  className,
  emptyMessage = "No instructions yet.",
  placeholder = "You are a helpful assistant...",
  compact = false,
}: AgentPromptPaneProps) {
  const [showSource, setShowSource] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const focusOnEdit = useRef(false);
  const { words, tokens } = promptStats(value);

  useEffect(() => {
    if (editing && focusOnEdit.current) {
      focusOnEdit.current = false;
      textareaRef.current?.focus();
    }
  }, [editing]);

  const handleEdit = () => {
    focusOnEdit.current = true;
    onEdit?.();
  };

  return (
    <section
      aria-label="Instructions"
      className={cn("flex min-w-0 flex-col bg-background", className)}
    >
      <div className="flex flex-wrap items-center gap-3 border-b px-4 py-2.5 sm:px-6">
        <h2 className="text-sm font-medium">Instructions</h2>
        <span className="font-mono text-xs text-muted-foreground">
          {words} {pluralize(words, "word")} · ~{formatTokens(tokens)} {pluralize(tokens, "token")}
        </span>
        {!editing && (
          <div className="ml-auto flex items-center gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => setShowSource((current) => !current)}
              aria-pressed={showSource}
            >
              {showSource ? "Rendered" : "Source"}
            </Button>
            {onEdit && (
              <Button variant="outline" size="sm" onClick={handleEdit}>
                <Pencil className="size-4" />
                Edit prompt
              </Button>
            )}
          </div>
        )}
      </div>

      {editing ? (
        <div className="flex flex-1 flex-col gap-4 px-4 py-4 sm:px-6">
          <Textarea
            ref={textareaRef}
            id="system_prompt"
            aria-label="Instructions"
            aria-invalid={!!error}
            placeholder={placeholder}
            value={value}
            onChange={(event) => onChange?.(event.target.value)}
            className={cn(
              "min-h-[240px] flex-1 resize-y font-mono text-sm leading-relaxed md:text-sm",
              compact ? "lg:min-h-[240px]" : "lg:min-h-[520px]",
            )}
          />
          {error && <p className="text-xs text-destructive">{error}</p>}
          {checks}
        </div>
      ) : (
        <div
          className={cn(
            "min-h-[240px] flex-1 px-4 py-5 sm:px-6",
            compact ? "lg:min-h-0" : "lg:min-h-[520px]",
          )}
        >
          {!value.trim() ? (
            <p className="text-sm italic text-muted-foreground">{emptyMessage}</p>
          ) : showSource ? (
            <pre className="whitespace-pre-wrap break-words font-mono text-sm leading-relaxed">
              {value}
            </pre>
          ) : (
            <StreamdownMessage
              skipHtml={skipHtml}
              variant="compact"
              className="text-[15px] leading-relaxed"
            >
              {value}
            </StreamdownMessage>
          )}
        </div>
      )}
    </section>
  );
}
