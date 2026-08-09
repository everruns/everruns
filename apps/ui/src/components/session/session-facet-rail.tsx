"use client";

// Facet rail for the operational Sessions list (EVE-853).
//
// Every count comes from `GET /v1/sessions/facets`, aggregated server-side over
// the same predicate as the list. Nothing here is derived from the rows on
// screen — a page of 20 cannot know how many failures exist in 4,000 runs.
//
// Each dimension is counted with the other filters applied but its own
// selection excluded, which is what makes multi-select usable: picking `failed`
// does not zero out every other status in the Status facet.

import { useState } from "react";
import { Check, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RailSection } from "@/components/layout";
import { cn } from "@/lib/utils";
import { activityLabel, sourceLabel, type SessionFilters } from "@/lib/session-filters";
import { isViewActive, type SavedSessionView } from "@/lib/session-views";
import type { SessionFacetCount } from "@/lib/api/types";

interface FacetListProps {
  label: string;
  counts: SessionFacetCount[];
  selected: string[];
  labelFor: (value: string) => string;
  onToggle: (value: string) => void;
  /** Shown in place of the list when the filtered set has nothing in this dimension. */
  emptyHint: string;
}

function FacetList({ label, counts, selected, labelFor, onToggle, emptyHint }: FacetListProps) {
  // A selected value must stay clickable even when the current filter set
  // leaves it with no rows — otherwise the only way to undo it is the URL.
  const missingSelected = selected
    .filter((value) => !counts.some((count) => count.value === value))
    .map((value) => ({ value, count: 0 }));
  const entries = [...counts, ...missingSelected];

  return (
    <RailSection label={label}>
      {entries.length === 0 ? (
        <p className="text-[13px] text-muted-foreground">{emptyHint}</p>
      ) : (
        <div className="flex flex-col gap-1.5 text-[13px]">
          {entries.map((entry) => {
            const isSelected = selected.includes(entry.value);
            return (
              <button
                key={entry.value}
                type="button"
                aria-pressed={isSelected}
                onClick={() => onToggle(entry.value)}
                className={cn(
                  "flex items-center justify-between gap-2 text-left transition-colors hover:text-foreground",
                  isSelected ? "text-foreground" : "text-muted-foreground",
                )}
              >
                <span className="inline-flex min-w-0 items-center gap-1.5">
                  <Check
                    className={cn("size-3.5 flex-none", isSelected ? "opacity-100" : "opacity-0")}
                  />
                  <span className="truncate">{labelFor(entry.value)}</span>
                </span>
                <span className="flex-none font-mono text-muted-foreground">{entry.count}</span>
              </button>
            );
          })}
        </div>
      )}
    </RailSection>
  );
}

export interface SessionFacetRailProps {
  filters: SessionFilters;
  /** Facet buckets, or undefined while loading / after an error. */
  facets?: {
    by_activity: SessionFacetCount[];
    by_source: SessionFacetCount[];
    by_agent: SessionFacetCount[];
  };
  /** Failure loading the facets; the rail says so rather than showing zeros. */
  error?: Error | null;
  /** Resolve an agent's public id to a display name. */
  agentLabel: (agentId: string) => string;
  onToggleStatus: (value: string) => void;
  onToggleSource: (value: string) => void;
  onToggleAgent: (agentId: string) => void;
  views: SavedSessionView[];
  onSelectView: (view: SavedSessionView) => void;
  onSaveView: (name: string) => void;
  onRemoveView: (id: string) => void;
  /** False for the suggested defaults, which nobody owns and nobody can delete. */
  viewsAreSaved: boolean;
  canSaveView: boolean;
}

export function SessionFacetRail({
  filters,
  facets,
  error,
  agentLabel,
  onToggleStatus,
  onToggleSource,
  onToggleAgent,
  views,
  onSelectView,
  onSaveView,
  onRemoveView,
  viewsAreSaved,
  canSaveView,
}: SessionFacetRailProps) {
  const [naming, setNaming] = useState(false);
  const [draftName, setDraftName] = useState("");

  const submitName = () => {
    const name = draftName.trim();
    if (!name) return;
    onSaveView(name);
    setDraftName("");
    setNaming(false);
  };

  return (
    <>
      <RailSection label="Saved views">
        <div className="flex flex-col gap-1.5 text-[13px]">
          {views.map((view) => {
            const active = isViewActive(view, filters);
            return (
              <div key={view.id} className="group flex items-center justify-between gap-2">
                <button
                  type="button"
                  onClick={() => onSelectView(view)}
                  className={cn(
                    "min-w-0 truncate text-left transition-colors hover:text-foreground",
                    active ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  {view.name}
                </button>
                {viewsAreSaved && (
                  <button
                    type="button"
                    aria-label={`Remove ${view.name}`}
                    onClick={() => onRemoveView(view.id)}
                    className="flex-none text-muted-foreground/40 opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100"
                  >
                    <X className="size-3.5" />
                  </button>
                )}
              </div>
            );
          })}

          {naming ? (
            <div className="mt-1.5 flex items-center gap-1.5">
              <Input
                // Focused on mount: the field only exists because the user just
                // asked to name a view, so typing should land in it.
                ref={(node) => node?.focus()}
                value={draftName}
                placeholder="View name"
                onChange={(event) => setDraftName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") submitName();
                  if (event.key === "Escape") setNaming(false);
                }}
                className="h-7 text-[13px]"
              />
              <Button size="sm" variant="outline" onClick={submitName} disabled={!draftName.trim()}>
                Save
              </Button>
            </div>
          ) : (
            <button
              type="button"
              onClick={() => setNaming(true)}
              disabled={!canSaveView}
              className="mt-1.5 inline-flex items-center gap-1.5 text-muted-foreground transition-colors hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Plus className="size-3.5" />
              Save current filters
            </button>
          )}
        </div>
      </RailSection>

      {error ? (
        <RailSection label="Facets">
          <p className="text-[13px] text-destructive">Counts unavailable.</p>
        </RailSection>
      ) : (
        <>
          <FacetList
            label="Status"
            counts={facets?.by_activity ?? []}
            selected={filters.status}
            labelFor={activityLabel}
            onToggle={onToggleStatus}
            emptyHint="No runs match these filters."
          />
          <FacetList
            label="Source"
            counts={facets?.by_source ?? []}
            selected={filters.source}
            labelFor={sourceLabel}
            onToggle={onToggleSource}
            emptyHint="No runs match these filters."
          />
          <FacetList
            label="Agent"
            counts={facets?.by_agent ?? []}
            selected={filters.agent ? [filters.agent] : []}
            labelFor={agentLabel}
            onToggle={onToggleAgent}
            emptyHint="No agent-backed runs match these filters."
          />
        </>
      )}
    </>
  );
}
