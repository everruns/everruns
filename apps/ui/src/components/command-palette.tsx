/**
 * Global command palette (Cmd+K / Ctrl+K).
 *
 * Combines navigation and entity search in a single modal.
 * Keyboard navigation: Arrow Up/Down to move, Enter to select, Escape to close.
 * Results are grouped by category with section headers.
 */
"use client";

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useRouter, usePathname } from "next/navigation";
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
import { cn } from "@/lib/utils";
import { useCommandPalette } from "@/hooks/use-command-palette";
import {
  useGlobalSearch,
  type SearchResult,
  type SearchResultCategory,
} from "@/hooks/use-global-search";
import { Search, CornerDownLeft, ArrowUp, ArrowDown } from "lucide-react";

const CATEGORY_LABELS: Record<SearchResultCategory, string> = {
  navigation: "Pages",
  agent: "Agents",
  virtual_user: "Virtual Users",
  session: "Sessions",
  harness: "Harnesses",
  skill: "Skills",
  capability: "Capabilities",
  mcp_server: "MCP Servers",
  eval: "Evals",
  memory: "Memory",
  knowledge_index: "Knowledge Indexes",
  plugin: "Plugins",
  observer: "Observers",
  report: "Reports",
  organization: "Organizations",
  id: "Go to",
};

const CATEGORY_ORDER: SearchResultCategory[] = [
  "id",
  "navigation",
  "organization",
  "agent",
  "virtual_user",
  "session",
  "harness",
  "skill",
  "capability",
  "mcp_server",
  "eval",
  "memory",
  "knowledge_index",
  "plugin",
  "observer",
  "report",
];

function groupResults(results: SearchResult[]) {
  const groups: { key: string; label: string; items: SearchResult[] }[] = [];
  for (const category of CATEGORY_ORDER) {
    const categoryGroups = new Map<string, SearchResult[]>();
    for (const result of results.filter((result) => result.category === category)) {
      const label = result.navigationGroup ?? CATEGORY_LABELS[category];
      const items = categoryGroups.get(label) ?? [];
      items.push(result);
      categoryGroups.set(label, items);
    }
    for (const [label, items] of categoryGroups) {
      groups.push({ key: `${category}:${label}`, label, items });
    }
  }
  return groups;
}

export function CommandPalette() {
  const { open, setOpen } = useCommandPalette();
  const pathname = usePathname();

  // Close on route change
  useEffect(() => {
    setOpen(false);
  }, [pathname, setOpen]);

  return (
    <DialogPrimitive.Root open={open} onOpenChange={setOpen}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Backdrop className="data-[open]:animate-in data-[closed]:animate-out data-[closed]:fade-out-0 data-[open]:fade-in-0 data-[closed]:animation-duration-[200ms] fixed inset-0 z-50 bg-black/50" />
        <DialogPrimitive.Popup className="fixed inset-0 z-50 flex items-start justify-center px-3 pt-[12vh]">
          {/* Mount the search UI (and its data fetching) only while open, so we
              don't fetch every entity list on every page load. */}
          {open && <CommandPaletteContent setOpen={setOpen} />}
        </DialogPrimitive.Popup>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function CommandPaletteContent({ setOpen }: { setOpen: (open: boolean) => void }) {
  const router = useRouter();
  const listId = useId();
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const results = useGlobalSearch(query);
  const grouped = groupResults(results);

  // Flat list for keyboard navigation
  const flatResults = grouped.flatMap((g) => g.items);
  // Entity results arrive asynchronously and can remove the previously selected row.
  const activeIndex = Math.max(0, Math.min(selectedIndex, flatResults.length - 1));

  // Focus input once the palette mounts.
  useEffect(() => {
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  // Reset selected index when query changes.
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  const navigate = useCallback(
    (result: SearchResult) => {
      setOpen(false);
      if (result.onSelect) {
        result.onSelect();
        return;
      }
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
  }, [activeIndex, results]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex(Math.max(0, Math.min(activeIndex + 1, flatResults.length - 1)));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex(Math.max(activeIndex - 1, 0));
          break;
        case "Enter":
          e.preventDefault();
          if (flatResults[activeIndex]) {
            navigate(flatResults[activeIndex]);
          }
          break;
        case "Escape":
          e.preventDefault();
          setOpen(false);
          break;
      }
    },
    [flatResults, activeIndex, navigate, setOpen],
  );

  // Track which category boundary each flat index falls under
  let flatIndex = 0;

  return (
    <div className="bg-background w-full max-w-2xl border shadow-2xl animate-in fade-in-0 zoom-in-95 duration-150">
      <DialogPrimitive.Title className="sr-only">Search and navigate</DialogPrimitive.Title>
      <DialogPrimitive.Description className="sr-only">
        Find pages, resources, or organizations. Use arrow keys to choose a result and Enter to open
        it.
      </DialogPrimitive.Description>
      {/* Search input */}
      <div className="flex items-center gap-3 border-b px-4">
        <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Search pages, resources, organizations..."
          className="flex-1 bg-transparent py-3.5 text-sm outline-none placeholder:text-muted-foreground"
          aria-label="Search pages, resources, organizations"
          role="combobox"
          aria-expanded="true"
          aria-autocomplete="list"
          aria-controls={listId}
          aria-activedescendant={flatResults.length ? `${listId}-${activeIndex}` : undefined}
        />
        <kbd className="hidden sm:inline-flex h-5 items-center gap-1 border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground">
          ESC
        </kbd>
      </div>

      {/* Results */}
      <div
        ref={listRef}
        id={listId}
        role="listbox"
        aria-label="Search results"
        className="max-h-[min(50vh,400px)] overflow-y-auto p-1.5"
      >
        {flatResults.length === 0 && query.trim() ? (
          <div className="px-4 py-8 text-center text-sm text-muted-foreground">
            No results for &ldquo;{query}&rdquo;
          </div>
        ) : (
          grouped.map((group) => (
            <div key={group.key} role="group" aria-label={group.label}>
              <div className="px-3 py-1.5 text-[10px] font-medium uppercase tracking-[0.2em] text-muted-foreground">
                {group.label}
              </div>
              {group.items.map((result) => {
                const idx = flatIndex++;
                const isSelected = idx === activeIndex;
                return (
                  <button
                    key={result.id}
                    type="button"
                    role="option"
                    id={`${listId}-${idx}`}
                    tabIndex={-1}
                    aria-selected={isSelected}
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
                      {result.subtitle && !result.navigationGroup && (
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
  );
}
