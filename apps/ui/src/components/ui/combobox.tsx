"use client";

import * as React from "react";
import { Combobox as ComboboxPrimitive } from "@base-ui/react/combobox";
import { CheckIcon, ChevronDownIcon, XIcon } from "lucide-react";
import { cn } from "@/lib/utils";

export interface ComboboxOption {
  value: string;
  label: string;
  group?: string;
  disabled?: boolean;
}

interface ComboboxProps {
  options: ComboboxOption[];
  value: string;
  onValueChange: (value: string) => void;
  placeholder?: string;
  searchPlaceholder?: string;
  ariaLabel?: string;
  disabled?: boolean;
  className?: string;
  clearable?: boolean;
  /** Keep the current label visible when server search returns a different page. */
  valueLabel?: string;
  /** When supplied, the caller owns filtering and pagination. */
  onSearchChange?: (search: string) => void;
  loading?: boolean;
  error?: string;
  footer?: React.ReactNode;
}

export function Combobox({
  options,
  value,
  onValueChange,
  placeholder = "Select...",
  searchPlaceholder = "Search...",
  ariaLabel = placeholder,
  disabled = false,
  className,
  clearable = true,
  valueLabel,
  onSearchChange,
  loading = false,
  error,
  footer,
}: ComboboxProps) {
  const [search, setSearch] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  const selectedLabel = valueLabel ?? options.find((option) => option.value === value)?.label;
  const filtered = React.useMemo(() => {
    if (onSearchChange || !search.trim()) return options;
    const query = search.trim().toLowerCase();
    return options.filter(
      (option) =>
        option.label.toLowerCase().includes(query) || option.value.toLowerCase().includes(query),
    );
  }, [options, search, onSearchChange]);
  function updateSearch(next: string) {
    setSearch(next);
    onSearchChange?.(next);
  }
  return (
    <div className={cn("relative flex items-center", className)}>
      <ComboboxPrimitive.Root<string>
        items={filtered.map((option) => option.value)}
        value={value || null}
        onValueChange={(next) => {
          if (next !== null) onValueChange(next);
        }}
        inputValue={search}
        onInputValueChange={updateSearch}
        onOpenChange={(open) => {
          if (!open) updateSearch("");
        }}
        filter={null}
        autoHighlight
        disabled={disabled}
      >
        <ComboboxPrimitive.Trigger
          aria-label={ariaLabel}
          className="border-input focus-visible:border-ring focus-visible:ring-ring/50 flex h-8 w-full min-w-0 items-center justify-between gap-2 border bg-transparent px-2.5 py-1.5 text-[13px] shadow-xs outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50"
        >
          <span className={cn("truncate", !selectedLabel && "text-muted-foreground")}>
            {selectedLabel || placeholder}
          </span>
          <ChevronDownIcon
            className={cn("size-4 shrink-0 opacity-50", clearable && value && "mr-5")}
          />
        </ComboboxPrimitive.Trigger>
        {clearable && value && !disabled && (
          <button
            type="button"
            aria-label={`Clear ${ariaLabel}`}
            onClick={() => onValueChange("")}
            className="absolute right-2 text-muted-foreground hover:text-foreground"
          >
            <XIcon className="size-3.5" />
          </button>
        )}
        <ComboboxPrimitive.Portal>
          <ComboboxPrimitive.Positioner sideOffset={5} className="z-50">
            <ComboboxPrimitive.Popup
              initialFocus={inputRef}
              className="w-(--anchor-width) max-w-(--available-width) overflow-hidden border bg-popover text-popover-foreground shadow-md"
            >
              <div className="border-b p-2">
                <ComboboxPrimitive.Input
                  ref={inputRef}
                  aria-label={searchPlaceholder}
                  placeholder={searchPlaceholder}
                  className="w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
                />
              </div>
              <ComboboxPrimitive.List
                aria-label={ariaLabel}
                aria-busy={loading}
                className="max-h-60 overflow-y-auto overscroll-contain p-1"
              >
                {!loading && !error && filtered.length === 0 && (
                  <div
                    role="status"
                    className="px-2 py-4 text-center text-sm text-muted-foreground"
                  >
                    No results found
                  </div>
                )}
                {filtered.map((option, index) => (
                  <React.Fragment key={option.value}>
                    {option.group && option.group !== filtered[index - 1]?.group && (
                      <div className="px-2 py-1 text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                        {option.group}
                      </div>
                    )}
                    <ComboboxPrimitive.Item
                      value={option.value}
                      disabled={option.disabled}
                      className="flex cursor-default items-center gap-2 px-2 py-1.5 text-sm select-none data-[highlighted]:bg-muted data-[selected]:font-medium data-[disabled]:opacity-50"
                    >
                      <span className="min-w-0 flex-1 break-words">{option.label}</span>
                      <ComboboxPrimitive.ItemIndicator>
                        <CheckIcon className="size-4" />
                      </ComboboxPrimitive.ItemIndicator>
                    </ComboboxPrimitive.Item>
                  </React.Fragment>
                ))}
              </ComboboxPrimitive.List>
              {loading && (
                <p role="status" className="p-2 text-sm text-muted-foreground">
                  Loading…
                </p>
              )}
              {error && (
                <p role="alert" className="p-2 text-sm text-destructive">
                  {error}
                </p>
              )}
              {footer && <div className="border-t p-1">{footer}</div>}
            </ComboboxPrimitive.Popup>
          </ComboboxPrimitive.Positioner>
        </ComboboxPrimitive.Portal>
      </ComboboxPrimitive.Root>
    </div>
  );
}
