"use client";

import { useState, useMemo, useCallback } from "react";
import { useCapabilities, useAgentCapabilities, useSetAgentCapabilities } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  ChevronUp,
  ChevronDown,
  Save,
  LucideIcon,
} from "lucide-react";
import type { Capability, CapabilityId } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
};

interface CapabilitySelectorProps {
  agentId: string;
}

export function CapabilitySelector({ agentId }: CapabilitySelectorProps) {
  const { data: allCapabilities, isLoading: capabilitiesLoading } = useCapabilities();
  const { data: agentCapabilities, isLoading: agentCapabilitiesLoading } = useAgentCapabilities(agentId);
  const setCapabilities = useSetAgentCapabilities();

  // Compute initial capabilities from server data
  const initialCapabilities = useMemo(() => {
    if (!agentCapabilities) return [];
    return agentCapabilities
      .sort((a, b) => a.position - b.position)
      .map((ac) => ac.capability_id);
  }, [agentCapabilities]);

  const [localCapabilities, setLocalCapabilities] = useState<CapabilityId[] | null>(null);

  // Use local state if user has made changes, otherwise use server data
  const selectedCapabilities = localCapabilities ?? initialCapabilities;
  const hasChanges = localCapabilities !== null &&
    JSON.stringify(localCapabilities) !== JSON.stringify(initialCapabilities);

  const handleToggle = useCallback((capabilityId: CapabilityId, checked: boolean) => {
    const current = localCapabilities ?? initialCapabilities;
    let newSelected: CapabilityId[];
    if (checked) {
      newSelected = [...current, capabilityId];
    } else {
      newSelected = current.filter((id) => id !== capabilityId);
    }
    setLocalCapabilities(newSelected);
  }, [localCapabilities, initialCapabilities]);

  const handleSave = useCallback(async () => {
    if (!localCapabilities) return;
    try {
      await setCapabilities.mutateAsync({
        agentId,
        request: { capabilities: localCapabilities },
      });
      setLocalCapabilities(null); // Reset to use server data
    } catch (error) {
      console.error("Failed to save capabilities:", error);
    }
  }, [agentId, localCapabilities, setCapabilities]);

  const moveUp = useCallback((index: number) => {
    if (index === 0) return;
    const current = localCapabilities ?? initialCapabilities;
    const newSelected = [...current];
    [newSelected[index - 1], newSelected[index]] = [newSelected[index], newSelected[index - 1]];
    setLocalCapabilities(newSelected);
  }, [localCapabilities, initialCapabilities]);

  const moveDown = useCallback((index: number) => {
    const current = localCapabilities ?? initialCapabilities;
    if (index === current.length - 1) return;
    const newSelected = [...current];
    [newSelected[index], newSelected[index + 1]] = [newSelected[index + 1], newSelected[index]];
    setLocalCapabilities(newSelected);
  }, [localCapabilities, initialCapabilities]);

  // Get capability info by ID
  const getCapabilityInfo = useCallback((id: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === id), [allCapabilities]);

  if (capabilitiesLoading || agentCapabilitiesLoading) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Capabilities</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="space-y-2">
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
          </div>
        </CardContent>
      </Card>
    );
  }

  // Filter to only show available capabilities
  const availableCapabilities = allCapabilities?.filter(
    (c) => c.status === "available"
  ) || [];

  const comingSoonCapabilities = allCapabilities?.filter(
    (c) => c.status === "coming_soon"
  ) || [];

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Capabilities</CardTitle>
        {hasChanges && (
          <Button
            size="sm"
            onClick={handleSave}
            disabled={setCapabilities.isPending}
          >
            <Save className="w-4 h-4 mr-2" />
            {setCapabilities.isPending ? "Saving..." : "Save"}
          </Button>
        )}
      </CardHeader>
      <CardContent className="space-y-4">
        {/* Selected capabilities with reordering */}
        {selectedCapabilities.length > 0 && (
          <div className="space-y-2">
            <p className="text-sm font-medium text-muted-foreground">
              Enabled ({selectedCapabilities.length})
            </p>
            {selectedCapabilities.map((capId, index) => {
              const cap = getCapabilityInfo(capId);
              if (!cap) return null;
              const IconComponent = cap.icon ? iconMap[cap.icon] || CircleOff : CircleOff;

              return (
                <div
                  key={capId}
                  className="flex items-center gap-2 p-2 rounded-md border bg-muted/50"
                >
                  <div className="flex flex-col gap-0.5">
                    <button
                      onClick={() => moveUp(index)}
                      disabled={index === 0}
                      className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                      aria-label="Move up"
                    >
                      <ChevronUp className="w-3 h-3" />
                    </button>
                    <button
                      onClick={() => moveDown(index)}
                      disabled={index === selectedCapabilities.length - 1}
                      className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                      aria-label="Move down"
                    >
                      <ChevronDown className="w-3 h-3" />
                    </button>
                  </div>
                  <span className="text-xs text-muted-foreground w-4">
                    {index + 1}
                  </span>
                  <IconComponent className="w-4 h-4" />
                  <span className="flex-1">{cap.name}</span>
                  <Checkbox
                    checked={true}
                    onCheckedChange={(checked) =>
                      handleToggle(capId, checked as boolean)
                    }
                  />
                </div>
              );
            })}
          </div>
        )}

        {/* Available capabilities not yet selected */}
        {availableCapabilities.filter(
          (c) => !selectedCapabilities.includes(c.id)
        ).length > 0 && (
          <div className="space-y-2">
            <p className="text-sm font-medium text-muted-foreground">
              Available
            </p>
            {availableCapabilities
              .filter((c) => !selectedCapabilities.includes(c.id))
              .map((cap) => {
                const IconComponent = cap.icon
                  ? iconMap[cap.icon] || CircleOff
                  : CircleOff;

                return (
                  <div
                    key={cap.id}
                    className="flex items-center gap-2 p-2 rounded-md border hover:bg-muted/50"
                  >
                    <IconComponent className="w-4 h-4" />
                    <div className="flex-1">
                      <p className="text-sm">{cap.name}</p>
                      <p className="text-xs text-muted-foreground">
                        {cap.description}
                      </p>
                    </div>
                    <Checkbox
                      checked={false}
                      onCheckedChange={(checked) =>
                        handleToggle(cap.id, checked as boolean)
                      }
                    />
                  </div>
                );
              })}
          </div>
        )}

        {/* Coming soon capabilities */}
        {comingSoonCapabilities.length > 0 && (
          <div className="space-y-2">
            <p className="text-sm font-medium text-muted-foreground">
              Coming Soon
            </p>
            {comingSoonCapabilities.map((cap) => {
              const IconComponent = cap.icon
                ? iconMap[cap.icon] || CircleOff
                : CircleOff;

              return (
                <div
                  key={cap.id}
                  className="flex items-center gap-2 p-2 rounded-md border opacity-60"
                >
                  <IconComponent className="w-4 h-4" />
                  <div className="flex-1">
                    <p className="text-sm">{cap.name}</p>
                    <p className="text-xs text-muted-foreground">
                      {cap.description}
                    </p>
                  </div>
                  <Badge variant="secondary">Coming Soon</Badge>
                </div>
              );
            })}
          </div>
        )}

        {selectedCapabilities.length === 0 && availableCapabilities.length === 0 && (
          <p className="text-center py-4 text-muted-foreground">
            No capabilities available
          </p>
        )}
      </CardContent>
    </Card>
  );
}
