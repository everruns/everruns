"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import { Terminal, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import type { CommandDescriptor } from "@/lib/api/types";

interface CommandAutocompleteProps {
  commands: CommandDescriptor[];
  inputValue: string;
  onSelect: (command: CommandDescriptor) => void;
  onDismiss: () => void;
  visible: boolean;
}

export function CommandAutocomplete({
  commands,
  inputValue,
  onSelect,
  onDismiss,
  visible,
}: CommandAutocompleteProps) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  // Filter commands based on input after "/"
  const query = inputValue.startsWith("/") ? inputValue.slice(1).toLowerCase() : "";
  const filtered = commands.filter(
    (cmd) =>
      cmd.name.toLowerCase().includes(query) || cmd.description.toLowerCase().includes(query),
  );

  // Reset selection when filter changes — derived inline, no effect needed.
  const prevQueryRef = useRef(query);
  if (prevQueryRef.current !== query) {
    prevQueryRef.current = query;
    setSelectedIndex(0);
  }

  // Scroll selected item into view
  useEffect(() => {
    if (!listRef.current) return;
    const items = listRef.current.querySelectorAll("[data-command-item]");
    items[selectedIndex]?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (!visible || filtered.length === 0) return;

      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => (i + 1) % filtered.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => (i - 1 + filtered.length) % filtered.length);
      } else if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        onSelect(filtered[selectedIndex]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
      }
    },
    [visible, filtered, selectedIndex, onSelect, onDismiss],
  );

  useEffect(() => {
    if (visible) {
      document.addEventListener("keydown", handleKeyDown, true);
      return () => document.removeEventListener("keydown", handleKeyDown, true);
    }
  }, [visible, handleKeyDown]);

  if (!visible || filtered.length === 0) return null;

  return (
    <div
      ref={listRef}
      className="absolute bottom-full left-0 right-0 z-50 mb-2 max-h-[240px] overflow-y-auto border border-border bg-card p-1"
    >
      {filtered.map((cmd, i) => (
        <button
          key={cmd.name}
          data-command-item
          type="button"
          className={cn(
            "flex w-full items-center gap-2 px-2 py-2 text-sm outline-none select-none",
            i === selectedIndex
              ? "border-l-2 border-l-accent bg-[hsl(var(--accent)/0.08)] text-foreground"
              : "text-popover-foreground",
          )}
          onMouseEnter={() => setSelectedIndex(i)}
          onMouseDown={(e) => {
            e.preventDefault(); // Keep textarea focus
            onSelect(cmd);
          }}
        >
          {cmd.source === "system" ? (
            <Terminal className="icon-sharp h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
          ) : (
            <Sparkles className="icon-sharp h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
          )}
          <span className="font-mono text-xs text-foreground">/{cmd.name}</span>
          <span className="text-xs text-muted-foreground truncate">{cmd.description}</span>
        </button>
      ))}
    </div>
  );
}

/**
 * Returns whether the command autocomplete should be visible based on input.
 * Visible when: input starts with "/" and cursor is still in the command token
 * (no space after the slash-word).
 */
export function shouldShowCommandAutocomplete(input: string): boolean {
  return /^\/\S*$/.test(input);
}
