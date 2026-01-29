/**
 * Hook for managing capability dependencies
 */

import { useMemo, useCallback } from "react";
import type { Capability, CapabilityId, AgentCapabilityConfig } from "@/lib/api/types";

interface UseCapabilityDependenciesProps {
  capabilities: Capability[];
  selected: AgentCapabilityConfig[];
}

interface UseCapabilityDependenciesReturn {
  /** Set of selected capability IDs */
  selectedIds: Set<CapabilityId>;
  /** Map of capability ID to Capability */
  capabilityMap: Map<CapabilityId, Capability>;
  /** Get capability by ID */
  getCapability: (id: CapabilityId) => Capability | undefined;
  /** Get all dependencies for a capability (recursively) */
  getAllDependencies: (capId: CapabilityId) => CapabilityId[];
  /** Get which selected capabilities depend on a given capability */
  getDependents: (capId: CapabilityId) => CapabilityId[];
  /** Check if a capability can be removed (no dependents) */
  canRemove: (capId: CapabilityId) => boolean;
}

export function useCapabilityDependencies({
  capabilities,
  selected,
}: UseCapabilityDependenciesProps): UseCapabilityDependenciesReturn {
  // Get selected capability IDs for quick lookup
  const selectedIds = useMemo(() => new Set(selected.map((c) => c.ref)), [selected]);

  // Create a map of capability ID to capability for fast lookup
  const capabilityMap = useMemo(() => {
    const map = new Map<CapabilityId, Capability>();
    for (const cap of capabilities) {
      map.set(cap.id, cap);
    }
    return map;
  }, [capabilities]);

  // Get capability info by ID
  const getCapability = useCallback(
    (id: CapabilityId): Capability | undefined => capabilityMap.get(id),
    [capabilityMap],
  );

  // Get all dependencies for a capability (recursively)
  const getAllDependencies = useCallback(
    (capId: CapabilityId, visited: Set<CapabilityId> = new Set()): CapabilityId[] => {
      if (visited.has(capId)) return []; // Prevent cycles
      visited.add(capId);

      const cap = getCapability(capId);
      if (!cap?.dependencies?.length) return [];

      const deps: CapabilityId[] = [];
      for (const depId of cap.dependencies) {
        // Add direct dependency
        if (!deps.includes(depId)) {
          deps.push(depId);
        }
        // Add transitive dependencies
        for (const transitiveDep of getAllDependencies(depId, visited)) {
          if (!deps.includes(transitiveDep)) {
            deps.push(transitiveDep);
          }
        }
      }
      return deps;
    },
    [getCapability],
  );

  // Check which selected capabilities depend on a given capability
  const getDependents = useCallback(
    (capId: CapabilityId): CapabilityId[] => {
      const dependents: CapabilityId[] = [];
      for (const selectedCap of selected) {
        if (selectedCap.ref === capId) continue;
        const deps = getAllDependencies(selectedCap.ref);
        if (deps.includes(capId)) {
          dependents.push(selectedCap.ref);
        }
      }
      return dependents;
    },
    [selected, getAllDependencies],
  );

  // Check if a capability can be removed (no dependents require it)
  const canRemove = useCallback(
    (capId: CapabilityId): boolean => {
      return getDependents(capId).length === 0;
    },
    [getDependents],
  );

  return {
    selectedIds,
    capabilityMap,
    getCapability,
    getAllDependencies,
    getDependents,
    canRemove,
  };
}
