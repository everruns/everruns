"use client";

import { useState } from "react";
import { Textarea } from "./textarea";
import { cn } from "@/lib/utils";
import { StreamdownMessage } from "@/components/chat/streamdown-message";

interface PromptEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  required?: boolean;
  id?: string;
  className?: string;
  disabled?: boolean;
}

export function PromptEditor({
  value,
  onChange,
  placeholder = "You are a helpful assistant...",
  required,
  id,
  className,
  disabled = false,
}: PromptEditorProps) {
  const [mode, setMode] = useState<"edit" | "preview">("edit");

  return (
    <div className={cn("space-y-2", className)}>
      <div className="flex gap-1 border-b">
        <button
          type="button"
          onClick={() => setMode("edit")}
          disabled={disabled}
          className={cn(
            "px-3 py-1.5 text-sm font-medium transition-colors",
            mode === "edit"
              ? "border-b-2 border-primary text-foreground"
              : "text-muted-foreground hover:text-foreground",
            disabled && "cursor-not-allowed opacity-50",
          )}
        >
          Edit
        </button>
        <button
          type="button"
          onClick={() => setMode("preview")}
          disabled={disabled}
          className={cn(
            "px-3 py-1.5 text-sm font-medium transition-colors",
            mode === "preview"
              ? "border-b-2 border-primary text-foreground"
              : "text-muted-foreground hover:text-foreground",
            disabled && "cursor-not-allowed opacity-50",
          )}
        >
          Preview
        </button>
      </div>

      {mode === "edit" ? (
        <Textarea
          id={id}
          placeholder={placeholder}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          required={required}
          disabled={disabled}
          className="min-h-[300px] font-mono text-sm"
        />
      ) : (
        <div className="min-h-[300px] border bg-muted/50 p-4 text-sm">
          {value ? (
            <StreamdownMessage variant="compact">{value}</StreamdownMessage>
          ) : (
            <p className="text-muted-foreground italic">Nothing to preview</p>
          )}
        </div>
      )}
    </div>
  );
}

interface MarkdownDisplayProps {
  content: string;
  className?: string;
}

export function MarkdownDisplay({ content, className }: MarkdownDisplayProps) {
  return <StreamdownMessage className={cn("text-sm", className)}>{content}</StreamdownMessage>;
}
