"use client";
import { useId, useState } from "react";
import { Checkbox } from "./checkbox";
import { SearchInput } from "./search-input";
import { useUiTranslations } from "@/providers/locale-provider";
import type { ComboboxOption } from "./combobox";

// Inline multi-selection for resource bindings. Keep missing selections removable.
export function ResourceMultiSelect({
  label,
  options,
  value,
  onChange,
  disabled,
  emptyMessage,
}: {
  label: string;
  options: ComboboxOption[];
  value: string[];
  onChange: (value: string[]) => void;
  disabled?: boolean;
  emptyMessage?: string;
}) {
  const ui = useUiTranslations();
  const id = useId();
  const [search, setSearch] = useState("");
  const missing = value.filter((v) => !options.some((o) => o.value === v));
  const all: ComboboxOption[] = [
    ...options,
    ...missing.map((v) => ({ value: v, label: v, description: ui("Unavailable or archived") })),
  ];
  const filtered = all.filter((o) =>
    `${o.label} ${o.value}`.toLowerCase().includes(search.toLowerCase()),
  );
  return (
    <fieldset className="min-w-0 space-y-2" disabled={disabled}>
      <legend className="text-sm font-medium">{label}</legend>
      <SearchInput
        aria-label={`${ui("Search")} ${label}`}
        placeholder={`${ui("Search")} ${label.toLowerCase()}`}
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        disabled={disabled}
      />
      <div className="max-h-48 space-y-1 overflow-y-auto border p-2">
        {filtered.map((o, i) => (
          <label
            key={o.value}
            htmlFor={`${id}-${i}`}
            className="flex cursor-pointer items-start gap-2 p-1 text-sm"
          >
            <Checkbox
              id={`${id}-${i}`}
              checked={value.includes(o.value)}
              disabled={disabled || (o.disabled && !value.includes(o.value))}
              onCheckedChange={(checked) =>
                onChange(
                  checked ? [...new Set([...value, o.value])] : value.filter((v) => v !== o.value),
                )
              }
            />
            <span className="min-w-0 break-words">
              {o.label}
              {o.description && (
                <span className="block text-xs text-muted-foreground">{o.description}</span>
              )}
            </span>
          </label>
        ))}
        {filtered.length === 0 && (
          <p role="status" className="p-2 text-sm text-muted-foreground">
            {search ? ui("No results found") : (emptyMessage ?? ui("No results found"))}
          </p>
        )}
      </div>
    </fieldset>
  );
}
