"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Brain,
  Wrench,
  Image,
  DollarSign,
  Layers,
  Info,
  ChevronDown,
  ChevronUp,
  Download,
  X,
  Trash2,
} from "lucide-react";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { formatTokens } from "@/lib/formatting";
import type { LlmModelWithProvider } from "@/lib/api/types";

function formatCost(cost: number): string {
  if (cost >= 100) {
    return `$${cost.toFixed(0)}`;
  }
  if (cost >= 1) {
    return `$${cost.toFixed(2)}`;
  }
  return `$${cost.toFixed(3)}`;
}

export function ModelRow({
  model,
  onDelete,
  onToggleInstalled,
  isTogglingInstalled,
}: {
  model: LlmModelWithProvider;
  onDelete: (id: string) => void;
  onToggleInstalled: (id: string, installed: boolean) => void;
  isTogglingInstalled: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const profile = model.profile;

  return (
    <div className="border overflow-hidden">
      <div className="flex items-center justify-between p-3">
        <div className="flex items-center gap-3">
          <ProviderIcon
            providerType={model.provider_type}
            size="sm"
            showBackground={false}
            className="text-muted-foreground"
          />
          <div>
            <div className="font-medium flex items-center gap-2">
              {model.display_name}
              {model.installed && (
                <Badge
                  variant="outline"
                  className="text-xs bg-green-50 text-green-700 border-green-200"
                >
                  Installed
                </Badge>
              )}
              {profile && (
                <Badge
                  variant="outline"
                  className="text-xs bg-blue-50 text-blue-700 border-blue-200"
                >
                  {profile.family}
                </Badge>
              )}
            </div>
            <div className="text-sm text-muted-foreground">
              {model.model_id} - {model.provider_name}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {/* Profile capability badges */}
          {profile && (
            <div className="flex gap-1">
              {profile.reasoning && (
                <Badge variant="secondary" className="text-xs" title="Reasoning">
                  <Brain className="h-3 w-3 mr-1" />
                  Reasoning
                </Badge>
              )}
              {profile.tool_call && (
                <Badge variant="secondary" className="text-xs" title="Tool Calling">
                  <Wrench className="h-3 w-3 mr-1" />
                  Tools
                </Badge>
              )}
              {profile.attachment && (
                <Badge variant="secondary" className="text-xs" title="Attachments">
                  <Image className="h-3 w-3 mr-1" />
                  Vision
                </Badge>
              )}
            </div>
          )}
          {/* Legacy capabilities (only show if no profile) */}
          {!profile && model.capabilities.length > 0 && (
            <div className="flex gap-1">
              {model.capabilities.slice(0, 2).map((cap) => (
                <Badge key={cap} variant="secondary" className="text-xs">
                  {cap}
                </Badge>
              ))}
              {model.capabilities.length > 2 && (
                <Badge variant="secondary" className="text-xs">
                  +{model.capabilities.length - 2}
                </Badge>
              )}
            </div>
          )}
          <Badge
            variant="outline"
            className={
              model.status === "active"
                ? "bg-green-100 text-green-800"
                : "bg-gray-100 text-gray-800"
            }
          >
            {model.status}
          </Badge>
          {/* Install / Uninstall toggle */}
          <Button
            variant={model.installed ? "outline" : "default"}
            size="sm"
            onClick={() => onToggleInstalled(model.id, !model.installed)}
            disabled={isTogglingInstalled}
            title={
              model.installed
                ? "Uninstall model (remove from UI pickers)"
                : "Install model (make available in UI pickers)"
            }
          >
            {model.installed ? (
              <>
                <X className="h-4 w-4 mr-1" />
                Uninstall
              </>
            ) : (
              <>
                <Download className="h-4 w-4 mr-1" />
                Install
              </>
            )}
          </Button>
          {profile && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setExpanded(!expanded)}
              title={expanded ? "Collapse" : "Expand profile details"}
            >
              {expanded ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive"
            onClick={() => onDelete(model.id)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Expanded profile details */}
      {expanded && profile && (
        <div className="border-t bg-muted/30 p-4">
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm mb-3">
            {/* Limits */}
            {profile.limits && (
              <div>
                <div className="font-medium text-muted-foreground mb-1 flex items-center gap-1">
                  <Layers className="h-3.5 w-3.5" />
                  Token Limits
                </div>
                <div>Context: {formatTokens(profile.limits.context)}</div>
                {profile.limits.input && <div>Input: {formatTokens(profile.limits.input)}</div>}
                <div>Output: {formatTokens(profile.limits.output)}</div>
              </div>
            )}

            {/* Cost */}
            {profile.cost && (
              <div>
                <div className="font-medium text-muted-foreground mb-1 flex items-center gap-1">
                  <DollarSign className="h-3.5 w-3.5" />
                  Cost / 1M Tokens
                </div>
                <div>Input: {formatCost(profile.cost.input)}</div>
                <div>Output: {formatCost(profile.cost.output)}</div>
                {profile.cost.cache_read && <div>Cache: {formatCost(profile.cost.cache_read)}</div>}
              </div>
            )}

            {/* Capabilities */}
            <div>
              <div className="font-medium text-muted-foreground mb-1 flex items-center gap-1">
                <Wrench className="h-3.5 w-3.5" />
                Capabilities
              </div>
              <div className="space-y-0.5">
                <div className={profile.tool_call ? "text-green-700" : "text-muted-foreground"}>
                  {profile.tool_call ? "\u2713" : "\u2717"} Tool Calling
                </div>
                <div
                  className={profile.structured_output ? "text-green-700" : "text-muted-foreground"}
                >
                  {profile.structured_output ? "\u2713" : "\u2717"} Structured Output
                </div>
                <div className={profile.reasoning ? "text-green-700" : "text-muted-foreground"}>
                  {profile.reasoning ? "\u2713" : "\u2717"} Reasoning
                </div>
                <div className={profile.attachment ? "text-green-700" : "text-muted-foreground"}>
                  {profile.attachment ? "\u2713" : "\u2717"} Attachments
                </div>
              </div>
            </div>

            {/* Info */}
            <div>
              <div className="font-medium text-muted-foreground mb-1 flex items-center gap-1">
                <Info className="h-3.5 w-3.5" />
                Model Info
              </div>
              {profile.knowledge && <div>Knowledge: {profile.knowledge}</div>}
              {profile.release_date && <div>Released: {profile.release_date}</div>}
              {profile.modalities && <div>Input: {profile.modalities.input.join(", ")}</div>}
            </div>
          </div>
          <div className="text-xs text-muted-foreground border-t pt-2 mt-2">
            Profile data from{" "}
            <a
              href="https://models.dev"
              target="_blank"
              rel="noopener noreferrer"
              className="underline hover:text-foreground"
            >
              models.dev
            </a>
          </div>
        </div>
      )}
    </div>
  );
}
