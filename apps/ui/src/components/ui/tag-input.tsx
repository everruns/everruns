"use client";

import { useState, type ClipboardEvent, type KeyboardEvent } from "react";
import { X } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

const MAX_RENDERED_TAGS = 100;

function parseTags(value: string): string[] {
  const tags: string[] = [];
  const seen = new Set<string>();
  let start = 0;

  while (tags.length < MAX_RENDERED_TAGS - 1) {
    const comma = value.indexOf(",", start);
    if (comma === -1) break;

    const tag = value.slice(start, comma).trim();
    if (tag.length > 0 && !seen.has(tag)) {
      tags.push(tag);
      seen.add(tag);
    }
    start = comma + 1;
  }

  // Keep excess input intact in one chip so persisted comma-heavy values cannot
  // create unbounded DOM nodes and editing does not silently discard their data.
  const remainder = value.slice(start).trim();
  if (remainder.length > 0 && !seen.has(remainder)) tags.push(remainder);

  return tags;
}

// The last chip can hold every tag past the cap, so neither its rendered text
// nor its accessible name may grow with the stored value. The chip truncates
// visually; this keeps the screen-reader label finite too.
function shortLabel(tag: string): string {
  return tag.length > 40 ? `${tag.slice(0, 39)}…` : tag;
}

interface TagInputProps {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  "aria-describedby"?: string;
}

export function TagInput({
  id,
  value,
  onChange,
  placeholder = "Add a tag…",
  disabled = false,
  className,
  "aria-describedby": ariaDescribedBy,
}: TagInputProps) {
  const [draft, setDraft] = useState("");
  const tags = parseTags(value);

  const updateTags = (nextTags: string[]) => onChange(nextTags.join(", "));

  const addTags = (input: string) => {
    const additions = parseTags(input);
    if (additions.length === 0) return;

    updateTags([...tags, ...additions.filter((tag) => !tags.includes(tag))]);
    setDraft("");
  };

  const removeTag = (tagToRemove: string) => {
    updateTags(tags.filter((tag) => tag !== tagToRemove));
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" || event.key === ",") {
      event.preventDefault();
      addTags(draft);
      return;
    }

    if (event.key === "Backspace" && draft.length === 0 && tags.length > 0) {
      removeTag(tags[tags.length - 1]);
    }
  };

  const handlePaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pastedValue = event.clipboardData.getData("text");
    if (!pastedValue.includes(",")) return;

    event.preventDefault();
    addTags(pastedValue);
  };

  return (
    <div
      className={cn(
        "border-input focus-within:border-ring focus-within:ring-ring/50 flex min-h-8 w-full flex-wrap items-center gap-1.5 border bg-transparent px-2 py-1 shadow-xs transition-[color,box-shadow] focus-within:ring-[3px]",
        disabled && "pointer-events-none cursor-not-allowed opacity-50",
        className,
      )}
    >
      {tags.map((tag) => (
        <Badge key={tag} variant="secondary" className="max-w-64 gap-1 pl-2 text-xs">
          <span className="truncate" title={tag}>
            {tag}
          </span>
          <button
            type="button"
            className="-mr-1 inline-flex size-4 items-center justify-center text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={(event) => {
              event.stopPropagation();
              removeTag(tag);
            }}
            disabled={disabled}
            aria-label={`Remove ${shortLabel(tag)}`}
          >
            <X className="size-3" />
          </button>
        </Badge>
      ))}
      <input
        id={id}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={handleKeyDown}
        onPaste={handlePaste}
        onBlur={() => addTags(draft)}
        placeholder={tags.length === 0 ? placeholder : undefined}
        disabled={disabled}
        aria-describedby={ariaDescribedBy}
        className="h-6 min-w-28 flex-1 bg-transparent px-0.5 text-[13px] outline-none placeholder:text-muted-foreground"
      />
    </div>
  );
}
