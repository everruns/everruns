"use client";

import { useState } from "react";
import Markdown from "react-markdown";
import { Textarea } from "./textarea";
import { cn } from "@/lib/utils";

// Lightweight markdown styles without @tailwindcss/typography
const markdownStyles = `
  .markdown-body h1 { font-size: 1.5em; font-weight: 700; margin: 1em 0 0.5em; }
  .markdown-body h2 { font-size: 1.25em; font-weight: 600; margin: 1em 0 0.5em; }
  .markdown-body h3 { font-size: 1.1em; font-weight: 600; margin: 1em 0 0.5em; }
  .markdown-body p { margin: 0.5em 0; }
  .markdown-body ul, .markdown-body ol { margin: 0.5em 0; padding-left: 1.5em; }
  .markdown-body li { margin: 0.25em 0; }
  .markdown-body code { background: var(--color-muted); padding: 0.2em 0.4em; border-radius: 3px; font-size: 0.9em; }
  .markdown-body pre { background: var(--color-muted); padding: 1em; border-radius: 6px; overflow-x: auto; margin: 0.5em 0; }
  .markdown-body pre code { background: none; padding: 0; }
  .markdown-body blockquote { border-left: 3px solid var(--color-border); padding-left: 1em; margin: 0.5em 0; color: var(--color-muted-foreground); }
  .markdown-body hr { border: none; border-top: 1px solid var(--color-border); margin: 1em 0; }
  .markdown-body a { color: var(--color-primary); text-decoration: underline; }
  .markdown-body strong { font-weight: 600; }
  .markdown-body em { font-style: italic; }
`;

interface PromptEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  required?: boolean;
  id?: string;
  className?: string;
}

export function PromptEditor({
  value,
  onChange,
  placeholder = "You are a helpful assistant...",
  required,
  id,
  className,
}: PromptEditorProps) {
  const [mode, setMode] = useState<"edit" | "preview">("edit");

  return (
    <div className={cn("space-y-2", className)}>
      <style>{markdownStyles}</style>
      <div className="flex gap-1 border-b">
        <button
          type="button"
          onClick={() => setMode("edit")}
          className={cn(
            "px-3 py-1.5 text-sm font-medium transition-colors",
            mode === "edit"
              ? "border-b-2 border-primary text-foreground"
              : "text-muted-foreground hover:text-foreground"
          )}
        >
          Edit
        </button>
        <button
          type="button"
          onClick={() => setMode("preview")}
          className={cn(
            "px-3 py-1.5 text-sm font-medium transition-colors",
            mode === "preview"
              ? "border-b-2 border-primary text-foreground"
              : "text-muted-foreground hover:text-foreground"
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
          className="min-h-[300px] font-mono text-sm"
        />
      ) : (
        <div className="markdown-body min-h-[300px] rounded-md border bg-muted/50 p-4 text-sm">
          {value ? (
            <Markdown>{value}</Markdown>
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
  return (
    <>
      <style>{markdownStyles}</style>
      <div className={cn("markdown-body text-sm bg-muted p-4 rounded-md", className)}>
        <Markdown>{content}</Markdown>
      </div>
    </>
  );
}
