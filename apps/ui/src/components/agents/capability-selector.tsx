"use client";

import { useState, useMemo, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
  DialogDescription,
} from "@/components/ui/dialog";
import {
  Plus,
  Search,
  ChevronUp,
  ChevronDown,
  X,
  ChevronRight,
  Plug,
} from "lucide-react";
import type { Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import { cn } from "@/lib/utils";
import { InlineMarkdown } from "@/components/ui/markdown";

interface CapabilitySelectorProps {
  /** All available capabilities */
  capabilities: Capability[];
  /** Currently selected capability IDs (order matters) */
  selected: CapabilityId[];
  /** Callback when selection changes */
  onChange: (selected: CapabilityId[]) => void;
  /** Whether the selector is disabled */
  disabled?: boolean;
  /** Label for the component */
  label?: string;
}

/**
 * A searchable, category-grouped capability selector designed for hundreds of capabilities.
 * Selected capabilities can be reordered. Uses a dialog for selection.
 */
export function CapabilitySelector({
  capabilities,
  selected,
  onChange,
  disabled,
  label = "Capabilities",
}: CapabilitySelectorProps) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [collapsedCategories, setCollapsedCategories] = useState<Set<string>>(
    new Set()
  );

  // Get capability info by ID
  const getCapability = useCallback(
    (id: CapabilityId): Capability | undefined =>
      capabilities.find((c) => c.id === id),
    [capabilities]
  );

  // Filter and group capabilities
  const { availableCapabilities, groupedCapabilities } = useMemo(() => {
    const available = capabilities.filter((c) => c.status === "available");
    const query = searchQuery.toLowerCase().trim();

    // Filter by search query
    const filtered = query
      ? available.filter(
          (c) =>
            c.name.toLowerCase().includes(query) ||
            c.description.toLowerCase().includes(query) ||
            c.id.toLowerCase().includes(query) ||
            (c.category && c.category.toLowerCase().includes(query))
        )
      : available;

    // Group by category
    const grouped = new Map<string, Capability[]>();
    for (const cap of filtered) {
      const category = cap.category || "General";
      const existing = grouped.get(category) || [];
      grouped.set(category, [...existing, cap]);
    }

    // Sort categories alphabetically, but put "General" last
    const sortedCategories = Array.from(grouped.keys()).sort((a, b) => {
      if (a === "General") return 1;
      if (b === "General") return -1;
      return a.localeCompare(b);
    });

    return {
      availableCapabilities: available,
      groupedCapabilities: sortedCategories.map((category) => ({
        category,
        capabilities: grouped.get(category)!,
      })),
    };
  }, [capabilities, searchQuery]);

  // Handlers
  const handleToggle = useCallback(
    (capabilityId: CapabilityId, checked: boolean) => {
      if (checked) {
        onChange([...selected, capabilityId]);
      } else {
        onChange(selected.filter((id) => id !== capabilityId));
      }
    },
    [selected, onChange]
  );

  const handleRemove = useCallback(
    (capabilityId: CapabilityId) => {
      onChange(selected.filter((id) => id !== capabilityId));
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

  const toggleCategory = useCallback((category: string) => {
    setCollapsedCategories((prev) => {
      const next = new Set(prev);
      if (next.has(category)) {
        next.delete(category);
      } else {
        next.add(category);
      }
      return next;
    });
  }, []);

  const selectedCount = selected.length;
  const availableCount = availableCapabilities.length;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">{label}</span>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger
            render={
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={disabled}
              >
                <Plus className="w-4 h-4 mr-1" />
                Add
              </Button>
            }
          />
          <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
            <DialogHeader>
              <DialogTitle>Select Capabilities</DialogTitle>
              <DialogDescription>
                {selectedCount} of {availableCount} capabilities selected
              </DialogDescription>
            </DialogHeader>

            {/* Search input */}
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
              <Input
                placeholder="Search capabilities..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-9"
                autoFocus
              />
            </div>

            {/* Capability list */}
            <div className="flex-1 overflow-y-auto min-h-0 -mx-6 px-6">
              {groupedCapabilities.length === 0 ? (
                <div className="text-center py-8 text-muted-foreground">
                  {searchQuery
                    ? "No capabilities match your search"
                    : "No capabilities available"}
                </div>
              ) : (
                <div className="space-y-4 py-2">
                  {groupedCapabilities.map(({ category, capabilities: caps }) => {
                    const isCollapsed = collapsedCategories.has(category);
                    const selectedInCategory = caps.filter((c) =>
                      selected.includes(c.id)
                    ).length;

                    return (
                      <div key={category}>
                        {/* Category header */}
                        <button
                          type="button"
                          onClick={() => toggleCategory(category)}
                          className="flex items-center gap-2 w-full py-1 text-sm font-medium text-muted-foreground hover:text-foreground"
                        >
                          <ChevronRight
                            className={cn(
                              "w-4 h-4 transition-transform",
                              !isCollapsed && "rotate-90"
                            )}
                          />
                          {category}
                          <Badge variant="secondary" className="ml-auto">
                            {selectedInCategory}/{caps.length}
                          </Badge>
                        </button>

                        {/* Category items */}
                        {!isCollapsed && (
                          <div className="ml-6 space-y-1 mt-1">
                            {caps.map((cap) => {
                              const IconComponent = getCapabilityIcon(cap.icon);
                              const isSelected = selected.includes(cap.id);

                              return (
                                <label
                                  key={cap.id}
                                  className={cn(
                                    "flex items-start gap-3 p-2 rounded-md cursor-pointer transition-colors",
                                    isSelected
                                      ? "bg-primary/10 border border-primary/20"
                                      : "hover:bg-muted/50"
                                  )}
                                >
                                  <Checkbox
                                    checked={isSelected}
                                    onCheckedChange={(checked) =>
                                      handleToggle(cap.id, checked as boolean)
                                    }
                                    className="mt-0.5"
                                  />
                                  <IconComponent className="w-4 h-4 mt-0.5 shrink-0" />
                                  <div className="flex-1 min-w-0">
                                    <div className="flex items-center gap-2 text-sm font-medium">
                                      {cap.name}
                                      {cap.is_mcp && (
                                        <Badge variant="outline" className="text-xs px-1.5 py-0 h-5 gap-1">
                                          <Plug className="w-3 h-3" />
                                          MCP
                                        </Badge>
                                      )}
                                    </div>
                                    <div className="text-xs text-muted-foreground line-clamp-2">
                                      <InlineMarkdown content={cap.description} />
                                    </div>
                                  </div>
                                </label>
                              );
                            })}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            {/* Footer with done button */}
            <div className="flex justify-end pt-4 border-t">
              <Button type="button" onClick={() => setDialogOpen(false)}>
                Done
              </Button>
            </div>
          </DialogContent>
        </Dialog>
      </div>

      {/* Selected capabilities list */}
      {selectedCount === 0 ? (
        <div className="text-sm text-muted-foreground py-4 text-center border rounded-md border-dashed">
          No capabilities selected
        </div>
      ) : (
        <div className="space-y-1">
          {selected.map((capId, index) => {
            const cap = getCapability(capId);
            if (!cap) return null;
            const IconComponent = getCapabilityIcon(cap.icon);

            return (
              <div
                key={capId}
                className="flex items-center gap-2 p-2 rounded-md border bg-muted/30 group"
              >
                {/* Reorder controls */}
                <div className="flex flex-col gap-0.5">
                  <button
                    type="button"
                    onClick={() => moveUp(index)}
                    disabled={index === 0 || disabled}
                    className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                    aria-label="Move up"
                  >
                    <ChevronUp className="w-3 h-3" />
                  </button>
                  <button
                    type="button"
                    onClick={() => moveDown(index)}
                    disabled={index === selected.length - 1 || disabled}
                    className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                    aria-label="Move down"
                  >
                    <ChevronDown className="w-3 h-3" />
                  </button>
                </div>

                {/* Position indicator */}
                <span className="text-xs text-muted-foreground w-4 text-center">
                  {index + 1}
                </span>

                {/* Icon and name */}
                <IconComponent className="w-4 h-4 shrink-0" />
                <span className="flex-1 text-sm truncate flex items-center gap-2">
                  {cap.name}
                  {cap.is_mcp && (
                    <Badge variant="outline" className="text-xs px-1 py-0 h-4 gap-0.5 shrink-0">
                      <Plug className="w-2.5 h-2.5" />
                      MCP
                    </Badge>
                  )}
                </span>

                {/* Remove button */}
                <button
                  type="button"
                  onClick={() => handleRemove(capId)}
                  disabled={disabled}
                  className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive p-1 transition-opacity"
                  aria-label="Remove capability"
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
            );
          })}
        </div>
      )}

      {/* Coming soon indicator */}
      {capabilities.some((c) => c.status === "coming_soon") && (
        <p className="text-xs text-muted-foreground">
          More capabilities coming soon
        </p>
      )}
    </div>
  );
}
