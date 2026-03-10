/**
 * Global command palette (Cmd+K / Ctrl+K).
 *
 * Combines navigation and entity search in a single modal.
 * Keyboard navigation: Arrow Up/Down to move, Enter to select, Escape to close.
 * Results are grouped by category with section headers.
 */
"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { cn } from "@/lib/utils";
import { useCommandPalette } from "@/hooks/use-command-palette";
import {
  useGlobalSearch,
  type SearchResult,
  type SearchResultCategory,
} from "@/hooks/use-global-search";
import { Dialog, DialogOverlay, DialogPortal } from "@/components/ui/dialog";
import { Search, CornerDownLeft, ArrowUp, ArrowDown } from "lucide-react";

const CATEGORY_LABELS: Record<SearchResultCategory, string> = {
  navigation: "Pages",
  agent: "Agents",
  session: "Sessions",
  harness: "Harnesses",
  skill: "Skills",
  id: "Go to",
};

const CATEGORY_ORDER: SearchResultCategory[] = [
  "id",
  "navigation",
  "agent",
  "session",
  "harness",
  "skill",
];

function groupResults(
  results: SearchResult[],
): { category: SearchResultCategory; items: SearchResult[] }[] {
  const groups = new Map<SearchResultCategory, SearchResult[]>();
  for (const result of results) {
    const existing = groups.get(result.category);
    if (existing) {
      existing.push(result);
    } else {
      groups.set(result.category, [result]);
    }
  }
  return CATEGORY_ORDER.filter((cat) => groups.has(cat)).map((cat) => ({
    category: cat,
    items: groups.get(cat)!,
  }));
}

export function CommandPalette() {
  const { open, setOpen } = useCommandPalette();
  const router = useRouter();
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const results = useGlobalSearch(query);
  const grouped = groupResults(results);

  // Flat list for keyboard navigation
  const flatResults = grouped.flatMap((g) => g.items);

  // Reset state when opening
  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      // Focus input after dialog renders
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  // Reset selected index when results change
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  const navigate = useCallback(
    (result: SearchResult) => {
      setOpen(false);
      router.push(result.href);
    },
    [router, setOpen],
  );

  // Scroll selected item into view
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const selected = list.querySelector("[data-selected='true']");
    if (selected) {
      selected.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex((prev) => Math.min(prev + 1, flatResults.length - 1));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((prev) => Math.max(prev - 1, 0));
          break;
        case "Enter":
          e.preventDefault();
          if (flatResults[selectedIndex]) {
            navigate(flatResults[selectedIndex]);
          }
          break;
        case "Escape":
          e.preventDefault();
          setOpen(false);
          break;
      }
    },
    [flatResults, selectedIndex, navigate, setOpen],
  );

  // Track which category boundary each flat index falls under
  let flatIndex = 0;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogPortal>
        <DialogOverlay />
        {/* eslint-disable-next-line jsx-a11y/no-static-element-interactions */}
        <div
          className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]"
          onKeyDown={handleKeyDown}
        >
          <div className="bg-background w-full max-w-lg border shadow-2xl animate-in fade-in-0 zoom-in-95 duration-150">
            {/* Search input */}
            <div className="flex items-center gap-3 border-b px-4">
              <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search pages, agents, sessions..."
                className="flex-1 bg-transparent py-3.5 text-sm outline-none placeholder:text-muted-foreground"
              />
              <kbd className="hidden sm:inline-flex h-5 items-center gap-1 border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground">
                ESC
              </kbd>
            </div>

            {/* Results */}
            <div ref={listRef} className="max-h-[min(50vh,400px)] overflow-y-auto p-1.5">
              {flatResults.length === 0 && query.trim() ? (
                <div className="px-4 py-8 text-center text-sm text-muted-foreground">
                  No results for &ldquo;{query}&rdquo;
                </div>
              ) : (
                grouped.map((group) => (
                  <div key={group.category}>
                    <div className="px-3 py-1.5 text-[10px] font-medium uppercase tracking-[0.2em] text-muted-foreground">
                      {CATEGORY_LABELS[group.category]}
                    </div>
                    {group.items.map((result) => {
                      const idx = flatIndex++;
                      const isSelected = idx === selectedIndex;
                      return (
                        <button
                          key={result.id}
                          type="button"
                          data-selected={isSelected}
                          className={cn(
                            "flex w-full items-center gap-3 px-3 py-2 text-sm transition-colors",
                            isSelected
                              ? "bg-accent text-accent-foreground"
                              : "text-foreground hover:bg-accent/50",
                          )}
                          onClick={() => navigate(result)}
                          onMouseEnter={() => setSelectedIndex(idx)}
                        >
                          <result.icon className="h-4 w-4 shrink-0 text-muted-foreground" />
                          <div className="flex-1 text-left min-w-0">
                            <span className="truncate block">{result.title}</span>
                            {result.subtitle && (
                              <span className="truncate block text-xs text-muted-foreground">
                                {result.subtitle}
                              </span>
                            )}
                          </div>
                          {isSelected && (
                            <CornerDownLeft className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                          )}
                        </button>
                      );
                    })}
                  </div>
                ))
              )}
            </div>

            {/* Footer hints */}
            <div className="flex items-center gap-4 border-t px-4 py-2 text-[11px] text-muted-foreground">
              <span className="inline-flex items-center gap-1">
                <ArrowUp className="h-3 w-3" />
                <ArrowDown className="h-3 w-3" />
                navigate
              </span>
              <span className="inline-flex items-center gap-1">
                <CornerDownLeft className="h-3 w-3" />
                select
              </span>
            </div>
          </div>
        </div>
      </DialogPortal>
    </Dialog>
  );
}
