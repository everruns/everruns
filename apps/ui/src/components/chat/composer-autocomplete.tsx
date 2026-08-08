"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

export interface ComposerAutocompleteItem {
  id: string;
}

export const COMPOSER_AUTOCOMPLETE_LISTBOX_ID = "composer-autocomplete-listbox";

interface ComposerAutocompleteProps<T extends ComposerAutocompleteItem> {
  emptyMessage: string;
  items: T[];
  label: string;
  onDismiss: () => void;
  onSelect: (item: T) => void;
  renderItem: (item: T) => React.ReactNode;
  visible: boolean;
}

export function ComposerAutocomplete<T extends ComposerAutocompleteItem>({
  emptyMessage,
  items,
  label,
  onDismiss,
  onSelect,
  renderItem,
  visible,
}: ComposerAutocompleteProps<T>) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const itemIds = items.map((item) => item.id).join("\0");

  useEffect(() => {
    setSelectedIndex(0);
  }, [itemIds, visible]);

  useEffect(() => {
    if (!listRef.current || items.length === 0) return;
    const options = listRef.current.querySelectorAll("[role='option']");
    options[selectedIndex]?.scrollIntoView({ block: "nearest" });
  }, [items.length, selectedIndex]);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (!visible) return;

      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onDismiss();
        return;
      }

      if (items.length === 0) return;
      if (event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        setSelectedIndex((index) => (index + 1) % items.length);
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        event.stopPropagation();
        setSelectedIndex((index) => (index - 1 + items.length) % items.length);
      } else if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        event.stopPropagation();
        onSelect(items[selectedIndex]);
      }
    },
    [items, onDismiss, onSelect, selectedIndex, visible],
  );

  useEffect(() => {
    if (!visible) return;
    document.addEventListener("keydown", handleKeyDown, true);
    return () => document.removeEventListener("keydown", handleKeyDown, true);
  }, [handleKeyDown, visible]);

  if (!visible) return null;

  return (
    <div
      ref={listRef}
      id={COMPOSER_AUTOCOMPLETE_LISTBOX_ID}
      role="listbox"
      aria-label={label}
      className="absolute bottom-full left-0 right-0 z-50 mb-2 max-h-[min(240px,40vh)] overflow-y-auto border border-border bg-card p-1 shadow-lg"
    >
      {items.length === 0 ? (
        <div role="status" className="px-3 py-2 text-sm text-muted-foreground">
          {emptyMessage}
        </div>
      ) : (
        items.map((item, index) => (
          <button
            key={item.id}
            id={`composer-option-${item.id}`}
            type="button"
            role="option"
            aria-selected={index === selectedIndex}
            className={cn(
              "flex w-full items-center gap-2 px-2 py-2 text-left text-sm outline-none select-none",
              index === selectedIndex
                ? "border-l-2 border-l-accent bg-[hsl(var(--accent)/0.08)] text-foreground"
                : "text-popover-foreground",
            )}
            onMouseEnter={() => setSelectedIndex(index)}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => onSelect(item)}
          >
            {renderItem(item)}
          </button>
        ))
      )}
    </div>
  );
}
