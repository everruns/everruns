"use client";

// Choosing which of a provider's models to enable, in one place.
//
// The same picker serves three moments: the last step of connecting a provider,
// reviewing what a sync discovered, and changing a provider's enabled set later.
// Rows are grouped by service, recommended ones start ticked (see
// `recommendedModelIds`), and the footer applies the whole selection at once, so
// a catalog of hundreds never has to be enabled row by row.

import { useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { SearchInput } from "@/components/ui/search-input";
import { ModelIcon } from "@/components/models/model-icon";
import type { ModelWithProvider } from "@/lib/api/types";
import {
  SERVICE_LABELS,
  compareByRecency,
  groupByService,
  recommendedModelIds,
} from "@/lib/model-selection";

/** The starting selection: what is enabled, plus recommendations when asked. */
export function initialSelection(
  models: ModelWithProvider[],
  { recommend }: { recommend: boolean },
): Set<string> {
  const selected = new Set(models.filter((model) => model.enabled).map((model) => model.id));
  if (recommend) for (const id of recommendedModelIds(models)) selected.add(id);
  return selected;
}

export function ModelSelectionList({
  models,
  selected,
  onSelectedChange,
  showProvider = false,
}: {
  models: ModelWithProvider[];
  selected: Set<string>;
  onSelectedChange: (next: Set<string>) => void;
  showProvider?: boolean;
}) {
  const [search, setSearch] = useState("");
  const recommended = useMemo(() => recommendedModelIds(models), [models]);
  const groups = useMemo(() => {
    const query = search.trim().toLowerCase();
    const visible = models
      .filter(
        (model) =>
          !query ||
          `${model.display_name} ${model.model_id} ${model.provider_name}`
            .toLowerCase()
            .includes(query),
      )
      // Recommended rows first so the ticked set is visible without scrolling.
      .sort(
        (a, b) =>
          Number(recommended.has(b.id)) - Number(recommended.has(a.id)) || compareByRecency(a, b),
      );
    return groupByService(visible);
  }, [models, search, recommended]);

  const setMany = (ids: string[], on: boolean) => {
    const next = new Set(selected);
    for (const id of ids) {
      if (on) next.add(id);
      else next.delete(id);
    }
    onSelectedChange(next);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <SearchInput
        placeholder="Search models…"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
        aria-label="Search models"
      />
      <div className="-mx-6 min-h-0 flex-1 overflow-y-auto px-6">
        {groups.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            {search ? "No models match your search." : "No models to choose from."}
          </p>
        ) : (
          groups.map((group) => {
            const ids = group.models.map((model) => model.id);
            const chosen = ids.filter((id) => selected.has(id)).length;
            const all = chosen === ids.length;
            return (
              <section
                key={group.service}
                className="mb-4"
                aria-label={SERVICE_LABELS[group.service]}
              >
                <div className="sticky top-0 z-10 flex items-center justify-between gap-2 border-b bg-background py-2">
                  <span className="text-sm font-semibold">
                    {SERVICE_LABELS[group.service]}{" "}
                    <span className="font-mono text-xs font-normal text-muted-foreground">
                      {chosen}/{ids.length}
                    </span>
                  </span>
                  <Button variant="ghost" size="sm" onClick={() => setMany(ids, !all)}>
                    {all ? "Clear" : "Select all"}
                  </Button>
                </div>
                <ul className="divide-y">
                  {group.models.map((model) => {
                    const checked = selected.has(model.id);
                    const checkboxId = `pick-${model.id}`;
                    return (
                      <li key={model.id} className="flex items-center gap-3 py-2">
                        <Checkbox
                          id={checkboxId}
                          checked={checked}
                          onCheckedChange={(on) => setMany([model.id], on)}
                        />
                        <label
                          htmlFor={checkboxId}
                          className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                        >
                          <ModelIcon model={model} size="sm" showBackground={false} />
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-sm font-medium">
                              {model.display_name}
                              {showProvider && (
                                <span className="font-normal text-muted-foreground">
                                  {" "}
                                  ({model.provider_name})
                                </span>
                              )}
                            </span>
                            <span className="block truncate font-mono text-xs text-muted-foreground">
                              {model.model_id}
                            </span>
                          </span>
                        </label>
                        {recommended.has(model.id) && <Badge variant="outline">Recommended</Badge>}
                        {model.profile?.decisions && (
                          <Badge
                            variant={model.profile.decisions.calibrated ? "success" : "outline"}
                          >
                            {model.profile.decisions.calibrated ? "Calibrated" : "Uncalibrated"}
                          </Badge>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })
        )}
      </div>
    </div>
  );
}

/** Footer label: "Enable N models", or what will change when editing a set. */
export function selectionActionLabel(
  models: ModelWithProvider[],
  selected: Set<string>,
  mode: "enable" | "edit",
): string {
  const count = models.filter((model) => selected.has(model.id)).length;
  if (mode === "enable") {
    return count === 0 ? "Skip for now" : `Enable ${count} ${count === 1 ? "model" : "models"}`;
  }
  return `Save · ${count} enabled`;
}

/**
 * Drawer around {@link ModelSelectionList}. The caller decides what applying
 * means (enable the diff, mark reviewed); this only collects the selection.
 */
export function ModelSelectionDrawer({
  open,
  onOpenChange,
  title,
  description,
  models,
  mode,
  recommend,
  showProvider,
  onApply,
  applying,
  error,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  models: ModelWithProvider[];
  mode: "enable" | "edit";
  recommend: boolean;
  showProvider?: boolean;
  onApply: (selected: Set<string>) => void;
  applying: boolean;
  error?: string | null;
}) {
  return (
    <Drawer open={open} onOpenChange={onOpenChange}>
      <DrawerContent className="sm:max-w-xl">
        {open && (
          <ModelSelectionBody
            title={title}
            description={description}
            models={models}
            mode={mode}
            recommend={recommend}
            showProvider={showProvider}
            onApply={onApply}
            onCancel={() => onOpenChange(false)}
            applying={applying}
            error={error}
          />
        )}
      </DrawerContent>
    </Drawer>
  );
}

// Mounted per open, so the starting selection is recomputed from fresh data.
function ModelSelectionBody({
  title,
  description,
  models,
  mode,
  recommend,
  showProvider,
  onApply,
  onCancel,
  applying,
  error,
}: {
  title: string;
  description: string;
  models: ModelWithProvider[];
  mode: "enable" | "edit";
  recommend: boolean;
  showProvider?: boolean;
  onApply: (selected: Set<string>) => void;
  onCancel: () => void;
  applying: boolean;
  error?: string | null;
}) {
  const [selected, setSelected] = useState(() => initialSelection(models, { recommend }));
  return (
    <>
      <DrawerHeader className="p-0">
        <DrawerTitle>{title}</DrawerTitle>
        <DrawerDescription>{description}</DrawerDescription>
      </DrawerHeader>
      <ModelSelectionList
        models={models}
        selected={selected}
        onSelectedChange={setSelected}
        showProvider={showProvider}
      />
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <DrawerFooter className="flex-row justify-end gap-2 p-0">
        <Button variant="outline" onClick={onCancel} disabled={applying}>
          Cancel
        </Button>
        <Button onClick={() => onApply(selected)} disabled={applying}>
          {applying ? "Saving…" : selectionActionLabel(models, selected, mode)}
        </Button>
      </DrawerFooter>
    </>
  );
}
