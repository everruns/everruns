"use client";

import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useRef } from "react";

type FilterValue = string | number | boolean | null | readonly string[];
type PageFilters = Partial<
  Record<
    | "usage"
    | "service"
    | "dataset"
    | "dimension"
    | "measure"
    | "show_deprecated"
    | "source"
    | "mine"
    | "search"
    | "status"
    | "sort"
    | "direction"
    | "group_by"
    | "show_archived"
    | "agent_id"
    | "harness_id"
    | "provider_id"
    | "selected_id"
    | "category"
    | "type"
    | "from"
    | "to"
    | "range"
    | "page"
    | "offset"
    | "limit"
    | "tab"
    | "file_path"
    | "filter",
    FilterValue
  >
>;
type Entry = { kind: "filters" | "entity" | "rows"; get: () => Record<string, unknown> };
type Registry = { route: string; register: (route: string, entry: Entry) => () => void };
export const PageViewRegistryContext = createContext<Registry | null>(null);

// The provider is mounted only around the right page. A Chat transcript, hidden
// sidebar, password input or arbitrary DOM text cannot register itself as page data.
export function usePageViewRegistry(route: string) {
  const entries = useRef(new Map<string, Entry & { route: string }>());
  const register = useCallback((entryRoute: string, entry: Entry) => {
    const key = crypto.randomUUID();
    entries.current.set(key, { ...entry, route: entryRoute });
    return () => {
      entries.current.delete(key);
    };
  }, []);
  const registry = useMemo(() => ({ route, register }), [route, register]);
  const read = useCallback((currentRoute: string) => {
    const filters: Record<string, FilterValue> = {};
    const visible: Array<{ id: string; name?: string }> = [];
    const sections: Array<Record<string, unknown>> = [];
    let rowCharacters = 0;
    const keys = [
      "usage",
      "service",
      "dataset",
      "dimension",
      "measure",
      "show_deprecated",
      "source",
      "mine",
      "search",
      "status",
      "sort",
      "direction",
      "group_by",
      "show_archived",
      "agent_id",
      "harness_id",
      "provider_id",
      "selected_id",
      "category",
      "type",
      "from",
      "to",
      "range",
      "page",
      "offset",
      "limit",
      "tab",
      "file_path",
      "filter",
    ];
    for (const entry of entries.current.values()) {
      if (entry.route !== currentRoute || (entry.kind === "entity" && visible.length >= 50))
        continue;
      const value = entry.get();
      if (entry.kind === "entity") {
        if (typeof value.id === "string")
          visible.push({
            id: value.id.slice(0, 128),
            ...(typeof value.name === "string" ? { name: value.name.slice(0, 192) } : {}),
          });
      } else if (entry.kind === "rows") {
        if (sections.length >= 8) continue;
        const rows: Array<Record<string, string | number | boolean | null>> = [];
        const source = Array.isArray(value.rows) ? value.rows : [];
        for (const row of source.slice(0, 50)) {
          const fields: Record<string, string | number | boolean | null> = {};
          for (const [key, item] of Object.entries(row).slice(0, 24)) {
            if (["__proto__", "constructor", "prototype"].includes(key)) continue;
            if (typeof item === "string") fields[key.slice(0, 64)] = item.slice(0, 256);
            else if (
              item === null ||
              typeof item === "boolean" ||
              (typeof item === "number" && Number.isFinite(item))
            )
              fields[key.slice(0, 64)] = item;
          }
          const size = JSON.stringify(fields).length;
          if (rowCharacters + size > 12288) break;
          rowCharacters += size;
          rows.push(fields);
        }
        sections.push({
          label: String(value.label).slice(0, 128),
          load_state: value.load_state,
          rows,
          returned_rows: source.length,
          truncated: rows.length < source.length,
        });
      } else {
        for (const key of keys) {
          const item = value[key];
          if (typeof item === "string") filters[key] = item.slice(0, 512);
          else if (
            typeof item === "boolean" ||
            item === null ||
            (typeof item === "number" && Number.isFinite(item))
          )
            filters[key] = item;
          else if (Array.isArray(item) && item.every((v) => typeof v === "string"))
            filters[key] = item.slice(0, 50).map((v) => v.slice(0, 128));
        }
      }
    }
    return {
      filters,
      visible_entities: visible,
      visible_entities_limit: 50,
      sections,
      section_limits: { sections: 8, rows: 50, fields: 24, text: 256, characters: 12288 },
    };
  }, []);
  return { registry, read };
}

function useViewEntry(kind: Entry["kind"], value: Record<string, unknown>) {
  const registry = useContext(PageViewRegistryContext);
  const current = useRef(value);
  useLayoutEffect(() => {
    current.current = value;
  });
  useLayoutEffect(
    () => registry?.register(registry.route, { kind, get: () => current.current }),
    [registry, kind],
  );
}

export function useChatPageFilters(value: PageFilters) {
  useViewEntry("filters", value);
}

// Explicit projections of the rows actually displayed by the page. Callers
// select public scalar fields; no cached payload, credential or DOM traversal.
export function useChatPageRows(
  label: string,
  rows: readonly Record<string, string | number | boolean | null | undefined>[],
  options: { enabled?: boolean; loading?: boolean; error?: boolean } = {},
) {
  useViewEntry("rows", {
    label,
    rows: options.enabled === false || options.loading || options.error ? [] : rows,
    load_state:
      options.enabled === false
        ? "unavailable"
        : options.error
          ? "error"
          : options.loading
            ? "loading"
            : "ready",
  });
}

export function PageViewEntity({ id, name }: { id: string; name?: string }) {
  useViewEntry("entity", { id, name });
  return null;
}
