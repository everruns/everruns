"use client";

import { useState } from "react";
import { Textarea } from "./textarea";
import { cn } from "@/lib/utils";
import { StreamdownMessage } from "@/components/chat/streamdown-message";
import { SectionTabs } from "@/components/layout/page-layout";

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
      <SectionTabs
        value={mode}
        onValueChange={(value) => setMode(value as "edit" | "preview")}
        aria-label="Prompt editor"
        items={[
          { value: "edit", label: "Edit", disabled },
          { value: "preview", label: "Preview", disabled },
        ]}
      />

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
