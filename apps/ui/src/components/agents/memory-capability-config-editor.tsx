"use client";

import { useMemo } from "react";
import { Combobox, type ComboboxOption } from "@/components/ui/combobox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useMemoryStores } from "@/hooks/use-memory-stores";

interface MemoryConfig {
  store?: string;
  passive_recall_count?: number;
}

interface MemoryCapabilityConfigEditorProps {
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

const STORE_ID_PATTERN = /^mst_[0-9a-f]{32}$/;

export function MemoryCapabilityConfigEditor({
  config,
  onChange,
  disabled,
}: MemoryCapabilityConfigEditorProps) {
  const typed = config as MemoryConfig;
  const { data: stores, isLoading, error } = useMemoryStores();

  const storeOptions = useMemo<ComboboxOption[]>(() => {
    const options: ComboboxOption[] = [{ value: "", label: "Default store" }];
    for (const store of stores ?? []) {
      const suffix = store.is_default ? " (default)" : "";
      options.push({ value: store.id, label: `${store.name}${suffix}` });
    }
    return options;
  }, [stores]);

  const isUnknownStore =
    !isLoading &&
    !error &&
    typed.store &&
    STORE_ID_PATTERN.test(typed.store) &&
    !(stores ?? []).some((s) => s.id === typed.store);

  const passiveCount = typed.passive_recall_count ?? 5;

  const update = (patch: Partial<MemoryConfig>) => {
    const next: MemoryConfig = { ...typed, ...patch };
    if (!next.store) delete next.store;
    if (next.passive_recall_count === undefined || next.passive_recall_count === 5) {
      delete next.passive_recall_count;
    }
    onChange(next as Record<string, unknown>);
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1">
        <Label className="text-xs font-normal text-muted-foreground">Memory store</Label>
        <Combobox
          options={storeOptions}
          value={typed.store ?? ""}
          onValueChange={(value) => update({ store: value || undefined })}
          placeholder={isLoading ? "Loading..." : "Default store"}
          searchPlaceholder="Search stores..."
          disabled={disabled || isLoading}
        />
        {isUnknownStore && (
          <p className="text-xs text-destructive">
            Selected store is no longer available in this organization.
          </p>
        )}
        {error && (
          <p className="text-xs text-destructive">Failed to load memory stores.</p>
        )}
        <p className="text-xs text-muted-foreground">
          When unset, the agent uses the org&apos;s default store, created on first use.
        </p>
      </div>

      <div className="space-y-1">
        <Label
          htmlFor="memory-passive-recall-count"
          className="text-xs font-normal text-muted-foreground"
        >
          Passive recall count
        </Label>
        <Input
          id="memory-passive-recall-count"
          type="number"
          min={0}
          max={50}
          value={passiveCount}
          disabled={disabled}
          className="h-8 w-32 text-sm"
          onChange={(event) => {
            const value = event.target.value;
            const parsed = Number(value);
            if (value === "" || !Number.isFinite(parsed)) {
              update({ passive_recall_count: undefined });
              return;
            }
            update({ passive_recall_count: Math.max(0, Math.min(50, Math.round(parsed))) });
          }}
        />
        <p className="text-xs text-muted-foreground">
          Memories auto-injected per turn. Set to 0 to disable passive recall.
        </p>
      </div>
    </div>
  );
}
