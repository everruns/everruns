"use client";

import { useState, type ClipboardEvent, type KeyboardEvent } from "react";
import { X } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

function parseTags(value: string): string[] {
  return value
    .split(",")
    .map((tag) => tag.trim())
    .filter((tag, index, tags) => tag.length > 0 && tags.indexOf(tag) === index);
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
        <Badge key={tag} variant="secondary" className="gap-1 pl-2 text-xs">
          {tag}
          <button
            type="button"
            className="-mr-1 inline-flex size-4 items-center justify-center text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={(event) => {
              event.stopPropagation();
              removeTag(tag);
            }}
            disabled={disabled}
            aria-label={`Remove ${tag}`}
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
