"use client";

import { useState, useCallback } from "react";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ChevronUp, ChevronDown, X, Plug, Lock, Settings } from "lucide-react";
import type { Capability, CapabilityId, AgentCapabilityConfig } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import { cn } from "@/lib/utils";
import { CapabilitySettingsEditor, hasCapabilitySettings } from "./capability-settings-editor";

interface SelectedCapabilityListProps {
  selected: AgentCapabilityConfig[];
  disabled?: boolean;
  getCapability: (id: CapabilityId) => Capability | undefined;
  getDependents: (capId: CapabilityId) => CapabilityId[];
  onRemove: (capabilityId: CapabilityId) => void;
  onConfigChange: (capabilityId: CapabilityId, newConfig: Record<string, unknown>) => void;
  onMoveUp: (index: number) => void;
  onMoveDown: (index: number) => void;
}

export function SelectedCapabilityList({
  selected,
  disabled,
  getCapability,
  getDependents,
  onRemove,
  onConfigChange,
  onMoveUp,
  onMoveDown,
}: SelectedCapabilityListProps) {
  // Track which capability settings are expanded
  const [expandedSettings, setExpandedSettings] = useState<Set<CapabilityId>>(new Set());

  const toggleSettings = useCallback((capabilityId: CapabilityId) => {
    setExpandedSettings((prev) => {
      const next = new Set(prev);
      if (next.has(capabilityId)) {
        next.delete(capabilityId);
      } else {
        next.add(capabilityId);
      }
      return next;
    });
  }, []);

  if (selected.length === 0) {
    return (
      <div className="text-sm text-muted-foreground py-4 text-center border border-dashed">
        No capabilities selected
      </div>
    );
  }

  return (
    <div className="space-y-1">
      {selected.map((capConfig, index) => {
        const cap = getCapability(capConfig.ref);
        if (!cap) return null;
        const IconComponent = getCapabilityIcon(cap.icon);
        const hasSettings = hasCapabilitySettings(capConfig.ref);
        const isSettingsExpanded = expandedSettings.has(capConfig.ref);
        const hasConfigValues = Object.keys(capConfig.config).length > 0;
        const dependents = getDependents(capConfig.ref);
        const isRequired = dependents.length > 0;

        return (
          <Collapsible
            key={capConfig.ref}
            open={isSettingsExpanded}
            onOpenChange={() => hasSettings && toggleSettings(capConfig.ref)}
          >
            <div className="border bg-muted/30 group">
              <div className="flex items-center gap-2 p-2">
                {/* Reorder controls */}
                <div className="flex flex-col gap-0.5">
                  <button
                    type="button"
                    onClick={() => onMoveUp(index)}
                    disabled={index === 0 || disabled}
                    className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                    aria-label="Move up"
                  >
                    <ChevronUp className="w-3 h-3" />
                  </button>
                  <button
                    type="button"
                    onClick={() => onMoveDown(index)}
                    disabled={index === selected.length - 1 || disabled}
                    className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                    aria-label="Move down"
                  >
                    <ChevronDown className="w-3 h-3" />
                  </button>
                </div>

                {/* Position indicator */}
                <span className="text-xs text-muted-foreground w-4 text-center">{index + 1}</span>

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
                  {isRequired && (
                    <Tooltip>
                      <TooltipTrigger>
                        <Badge
                          variant="secondary"
                          className="text-xs px-1 py-0 h-4 gap-0.5 shrink-0"
                        >
                          <Lock className="w-2.5 h-2.5" />
                          Required
                        </Badge>
                      </TooltipTrigger>
                      <TooltipContent>
                        <p>
                          Required by:{" "}
                          {dependents.map((d) => getCapability(d)?.name ?? d).join(", ")}
                        </p>
                      </TooltipContent>
                    </Tooltip>
                  )}
                </span>

                {/* Settings button (if capability has configurable settings) */}
                {hasSettings && (
                  <CollapsibleTrigger asChild>
                    <button
                      type="button"
                      disabled={disabled}
                      className={cn(
                        "text-muted-foreground hover:text-foreground p-1 transition-colors",
                        isSettingsExpanded && "text-foreground",
                        hasConfigValues && !isSettingsExpanded && "text-primary",
                      )}
                      aria-label="Toggle settings"
                    >
                      <Settings className="w-3.5 h-3.5" />
                    </button>
                  </CollapsibleTrigger>
                )}

                {/* Remove button */}
                {isRequired ? (
                  <Tooltip>
                    <TooltipTrigger>
                      <span className="text-muted-foreground/50 p-1 cursor-not-allowed">
                        <Lock className="w-3 h-3" />
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>
                      <p>
                        Cannot remove: required by{" "}
                        {dependents.map((d) => getCapability(d)?.name ?? d).join(", ")}
                      </p>
                    </TooltipContent>
                  </Tooltip>
                ) : (
                  <button
                    type="button"
                    onClick={() => onRemove(capConfig.ref)}
                    disabled={disabled}
                    className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive p-1 transition-opacity"
                    aria-label="Remove capability"
                  >
                    <X className="w-3 h-3" />
                  </button>
                )}
              </div>

              {/* Settings editor (collapsible) */}
              {hasSettings && (
                <CollapsibleContent>
                  <div className="px-2 pb-2 border-t border-border/50 mx-2 pt-2">
                    <CapabilitySettingsEditor
                      capabilityId={capConfig.ref}
                      config={capConfig.config}
                      onChange={(newConfig) => onConfigChange(capConfig.ref, newConfig)}
                      disabled={disabled}
                    />
                  </div>
                </CollapsibleContent>
              )}
            </div>
          </Collapsible>
        );
      })}
    </div>
  );
}
