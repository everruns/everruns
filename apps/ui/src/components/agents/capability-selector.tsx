"use client";

import { useCallback } from "react";
import type { Capability, CapabilityId, AgentCapabilityConfig } from "@/lib/api/types";
import { useCapabilityDependencies } from "./use-capability-dependencies";
import { CapabilityDialog } from "./capability-dialog";
import { SelectedCapabilityList } from "./selected-capability-list";

interface CapabilitySelectorProps {
  /** All available capabilities */
  capabilities: Capability[];
  /** Currently selected capabilities with configs (order matters) */
  selected: AgentCapabilityConfig[];
  /** Callback when selection changes */
  onChange: (selected: AgentCapabilityConfig[]) => void;
  /** Whether the selector is disabled */
  disabled?: boolean;
  /** Label for the component */
  label?: string;
}

/**
 * A searchable, category-grouped capability selector designed for hundreds of capabilities.
 * Selected capabilities can be reordered. Uses a dialog for selection.
 * Automatically handles capability dependencies.
 */
export function CapabilitySelector({
  capabilities,
  selected,
  onChange,
  disabled,
  label = "Capabilities",
}: CapabilitySelectorProps) {
  const {
    selectedIds,
    getCapability,
    getAllDependencies,
    getDependents,
    canRemove,
  } = useCapabilityDependencies({ capabilities, selected });

  // Handle toggling a capability (with automatic dependency handling)
  const handleToggle = useCallback(
    (capabilityId: CapabilityId, checked: boolean) => {
      if (checked) {
        // Add capability and its dependencies with empty configs
        const deps = getAllDependencies(capabilityId);
        const existingIds = new Set(selected.map((c) => c.ref));
        const toAdd: AgentCapabilityConfig[] = [];

        // Add dependencies first
        for (const depId of deps) {
          if (!existingIds.has(depId)) {
            toAdd.push({ ref: depId, config: {} });
          }
        }
        // Add the capability itself
        if (!existingIds.has(capabilityId)) {
          toAdd.push({ ref: capabilityId, config: {} });
        }

        onChange([...selected, ...toAdd]);
      } else {
        // Remove capability if no dependents require it
        if (canRemove(capabilityId)) {
          onChange(selected.filter((c) => c.ref !== capabilityId));
        }
      }
    },
    [selected, onChange, getAllDependencies, canRemove]
  );

  const handleRemove = useCallback(
    (capabilityId: CapabilityId) => {
      if (canRemove(capabilityId)) {
        onChange(selected.filter((c) => c.ref !== capabilityId));
      }
    },
    [selected, onChange, canRemove]
  );

  const handleConfigChange = useCallback(
    (capabilityId: CapabilityId, newConfig: Record<string, unknown>) => {
      onChange(
        selected.map((c) =>
          c.ref === capabilityId ? { ...c, config: newConfig } : c
        )
      );
    },
    [selected, onChange]
  );

  const moveUp = useCallback(
    (index: number) => {
      if (index === 0) return;
      const newSelected = [...selected];
      [newSelected[index - 1], newSelected[index]] = [
        newSelected[index],
        newSelected[index - 1],
      ];
      onChange(newSelected);
    },
    [selected, onChange]
  );

  const moveDown = useCallback(
    (index: number) => {
      if (index === selected.length - 1) return;
      const newSelected = [...selected];
      [newSelected[index], newSelected[index + 1]] = [
        newSelected[index + 1],
        newSelected[index],
      ];
      onChange(newSelected);
    },
    [selected, onChange]
  );

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">{label}</span>
        <CapabilityDialog
          capabilities={capabilities}
          selectedIds={selectedIds}
          disabled={disabled}
          getCapability={getCapability}
          getDependents={getDependents}
          onToggle={handleToggle}
        />
      </div>

      <SelectedCapabilityList
        selected={selected}
        disabled={disabled}
        getCapability={getCapability}
        getDependents={getDependents}
        onRemove={handleRemove}
        onConfigChange={handleConfigChange}
        onMoveUp={moveUp}
        onMoveDown={moveDown}
      />

      {/* Coming soon indicator */}
      {capabilities.some((c) => c.status === "coming_soon") && (
        <p className="text-xs text-muted-foreground">
          More capabilities coming soon
        </p>
      )}
    </div>
  );
}
