"use client";

import { useMemo } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useKnowledgeIndexes } from "@/hooks/use-knowledge-indexes";
import type { KnowledgeIndex } from "@/lib/api/types";

// Mirrors crates/core/src/capabilities/knowledge_index.rs: KnowledgeIndexConfig
// wire shape `{ indexes: string[], top_k?: number }`.
interface KnowledgeIndexConfig {
  indexes?: string[];
  top_k?: number;
}

interface KnowledgeIndexConfigEditorProps {
  config: unknown;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

const INDEX_ID_PATTERN = /^kidx_[0-9a-f]{32}$/;
export const MIN_TOP_K = 1;
export const MAX_TOP_K = 50;

export function readIndexes(config: unknown): string[] {
  if (!config || typeof config !== "object") return [];
  const typed = config as KnowledgeIndexConfig;
  if (!Array.isArray(typed.indexes)) return [];
  // De-duplicate while preserving order; the server rejects duplicates.
  return [...new Set(typed.indexes.filter((id): id is string => typeof id === "string"))];
}

export function readTopK(config: unknown): number | undefined {
  if (!config || typeof config !== "object") return undefined;
  const value = (config as KnowledgeIndexConfig).top_k;
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

// Validate top_k against the server bounds (1..=50). Returns null for an empty
// value so a cleared field does not render an error before save.
export function validateTopK(value: number | undefined): string | null {
  if (value === undefined) return null;
  if (!Number.isInteger(value) || value < MIN_TOP_K || value > MAX_TOP_K) {
    return `top_k must be between ${MIN_TOP_K} and ${MAX_TOP_K}`;
  }
  return null;
}

function writeConfig(indexes: string[], topK: number | undefined): Record<string, unknown> {
  const config: Record<string, unknown> = {};
  if (indexes.length > 0) config.indexes = indexes;
  if (topK !== undefined) config.top_k = topK;
  return config;
}

export function KnowledgeIndexConfigEditor({
  config,
  onChange,
  disabled,
}: KnowledgeIndexConfigEditorProps) {
  const selected = readIndexes(config);
  const topK = readTopK(config);
  const { data: indexes, isLoading, error } = useKnowledgeIndexes({ includeArchived: false });

  const activeIndexes = useMemo<KnowledgeIndex[]>(
    () => (indexes ?? []).filter((v) => v.status === "active"),
    [indexes],
  );

  // Selected IDs that no longer resolve to an active index (archived/removed)
  // are surfaced so the user can drop them. Suppressed while loading/errored.
  const unknownSelected = useMemo<string[]>(() => {
    if (isLoading || error) return [];
    return selected.filter((id) => !activeIndexes.some((v) => v.id === id));
  }, [selected, activeIndexes, isLoading, error]);

  const topKError = validateTopK(topK);

  const toggleIndex = (id: string, checked: boolean) => {
    const next = checked ? [...selected, id] : selected.filter((value) => value !== id);
    onChange(writeConfig([...new Set(next)], topK));
  };

  const updateTopK = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      onChange(writeConfig(selected, undefined));
      return;
    }
    const parsed = Number(trimmed);
    onChange(writeConfig(selected, Number.isFinite(parsed) ? parsed : topK));
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1">
        <Label className="text-xs font-normal text-muted-foreground">Knowledge Indexes</Label>
        <p className="text-xs text-muted-foreground">
          Select the indexes this capability can search.
        </p>
      </div>

      {error && <p className="text-xs text-destructive">Failed to load knowledge indexes.</p>}

      {!isLoading && activeIndexes.length === 0 && selected.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No active knowledge indexes in this org. Create one first to bind it.
        </p>
      )}

      {activeIndexes.length > 0 && (
        <div className="space-y-2">
          {activeIndexes.map((index) => {
            const checked = selected.includes(index.id);
            const checkboxId = `kidx-select-${index.id}`;
            return (
              <div
                key={index.id}
                className="flex items-center gap-2 rounded-md border border-border/60 p-2 text-sm"
                data-testid="knowledge-index-row"
              >
                <Checkbox
                  id={checkboxId}
                  checked={checked}
                  disabled={disabled}
                  onCheckedChange={(value) => toggleIndex(index.id, value)}
                />
                <Label htmlFor={checkboxId} className="min-w-0 flex-1 cursor-pointer font-normal">
                  <span className="block truncate">{index.name}</span>
                  <span className="block truncate font-mono text-xs text-muted-foreground">
                    {index.id}
                  </span>
                </Label>
              </div>
            );
          })}
        </div>
      )}

      {unknownSelected.length > 0 && (
        <div className="space-y-1.5">
          {unknownSelected.map((id) => (
            <div
              key={id}
              className="flex items-center gap-2 rounded-md border border-destructive/40 p-2 text-sm"
              data-testid="knowledge-index-unknown-row"
            >
              <Checkbox
                checked
                disabled={disabled}
                onCheckedChange={() => toggleIndex(id, false)}
              />
              <span className="min-w-0 flex-1">
                <span className="block font-mono text-xs">{id}</span>
                <span className="block text-xs text-destructive">Unavailable or archived</span>
              </span>
            </div>
          ))}
        </div>
      )}

      <div className="w-32 space-y-1">
        <Label htmlFor="kidx-top-k" className="text-xs font-normal text-muted-foreground">
          Top K
        </Label>
        <Input
          id="kidx-top-k"
          type="number"
          min={MIN_TOP_K}
          max={MAX_TOP_K}
          value={topK ?? ""}
          placeholder="10"
          disabled={disabled}
          className="h-8 text-sm"
          onChange={(event) => updateTopK(event.target.value)}
        />
        {topKError && <p className="text-xs text-destructive">{topKError}</p>}
      </div>

      {/* Surface any malformed configured IDs (defensive; server validates). */}
      {selected.some((id) => !INDEX_ID_PATTERN.test(id)) && (
        <p className="text-xs text-destructive">One or more bound index IDs are malformed.</p>
      )}
    </div>
  );
}
