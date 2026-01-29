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
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Plus, Search, ChevronRight, Plug, Link, Lock } from "lucide-react";
import type { Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import { cn } from "@/lib/utils";
import { InlineMarkdown } from "@/components/ui/markdown";

interface CapabilityDialogProps {
  capabilities: Capability[];
  selectedIds: Set<CapabilityId>;
  disabled?: boolean;
  getCapability: (id: CapabilityId) => Capability | undefined;
  getDependents: (capId: CapabilityId) => CapabilityId[];
  onToggle: (capabilityId: CapabilityId, checked: boolean) => void;
}

export function CapabilityDialog({
  capabilities,
  selectedIds,
  disabled,
  getCapability,
  getDependents,
  onToggle,
}: CapabilityDialogProps) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [collapsedCategories, setCollapsedCategories] = useState<Set<string>>(new Set());

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
            (c.category && c.category.toLowerCase().includes(query)),
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

  const selectedCount = selectedIds.size;
  const availableCount = availableCapabilities.length;

  return (
    <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
      <DialogTrigger
        render={
          <Button type="button" variant="outline" size="sm" disabled={disabled}>
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
              {searchQuery ? "No capabilities match your search" : "No capabilities available"}
            </div>
          ) : (
            <div className="space-y-4 py-2">
              {groupedCapabilities.map(({ category, capabilities: caps }) => {
                const isCollapsed = collapsedCategories.has(category);
                const selectedInCategory = caps.filter((c) => selectedIds.has(c.id)).length;

                return (
                  <div key={category}>
                    {/* Category header */}
                    <button
                      type="button"
                      onClick={() => toggleCategory(category)}
                      className="flex items-center gap-2 w-full py-1 text-sm font-medium text-muted-foreground hover:text-foreground"
                    >
                      <ChevronRight
                        className={cn("w-4 h-4 transition-transform", !isCollapsed && "rotate-90")}
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
                          const isSelected = selectedIds.has(cap.id);
                          const dependents = getDependents(cap.id);
                          const isRequired = dependents.length > 0;
                          const hasDependencies = (cap.dependencies?.length ?? 0) > 0;

                          return (
                            <label
                              key={cap.id}
                              className={cn(
                                "flex items-start gap-3 p-2 rounded-md cursor-pointer transition-colors",
                                isSelected
                                  ? "bg-primary/10 border border-primary/20"
                                  : "hover:bg-muted/50",
                              )}
                            >
                              <Checkbox
                                checked={isSelected}
                                onCheckedChange={(checked) => onToggle(cap.id, checked as boolean)}
                                disabled={isSelected && isRequired}
                                className="mt-0.5"
                              />
                              <IconComponent className="w-4 h-4 mt-0.5 shrink-0" />
                              <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2 text-sm font-medium">
                                  {cap.name}
                                  {cap.is_mcp && (
                                    <Badge
                                      variant="outline"
                                      className="text-xs px-1.5 py-0 h-5 gap-1"
                                    >
                                      <Plug className="w-3 h-3" />
                                      MCP
                                    </Badge>
                                  )}
                                  {hasDependencies && (
                                    <Tooltip>
                                      <TooltipTrigger>
                                        <Badge
                                          variant="secondary"
                                          className="text-xs px-1.5 py-0 h-5 gap-1"
                                        >
                                          <Link className="w-3 h-3" />
                                          Requires
                                        </Badge>
                                      </TooltipTrigger>
                                      <TooltipContent>
                                        <p>
                                          Depends on:{" "}
                                          {cap.dependencies
                                            ?.map((d) => getCapability(d)?.name ?? d)
                                            .join(", ")}
                                        </p>
                                      </TooltipContent>
                                    </Tooltip>
                                  )}
                                  {isRequired && (
                                    <Tooltip>
                                      <TooltipTrigger>
                                        <Badge
                                          variant="secondary"
                                          className="text-xs px-1.5 py-0 h-5 gap-1"
                                        >
                                          <Lock className="w-3 h-3" />
                                          Required
                                        </Badge>
                                      </TooltipTrigger>
                                      <TooltipContent>
                                        <p>
                                          Required by:{" "}
                                          {dependents
                                            .map((d) => getCapability(d)?.name ?? d)
                                            .join(", ")}
                                        </p>
                                      </TooltipContent>
                                    </Tooltip>
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
  );
}
