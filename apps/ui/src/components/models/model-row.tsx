"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  Brain,
  Wrench,
  Image,
  DollarSign,
  Layers,
  Info,
  ChevronDown,
  ChevronUp,
  Pencil,
  ToggleLeft,
  ToggleRight,
  Trash2,
} from "lucide-react";
import { ModelIcon } from "@/components/models/model-icon";
import { EditModelDialog } from "@/components/models/edit-model-dialog";
import { formatTokens } from "@/lib/formatting";
import type { ModelWithProvider, Provider, UpdateModelRequest } from "@/lib/api/types";

function formatCost(cost: number): string {
  if (cost >= 100) {
    return `$${cost.toFixed(0)}`;
  }
  if (cost >= 1) {
    return `$${cost.toFixed(2)}`;
  }
  return `$${cost.toFixed(3)}`;
}

function formatReleaseDate(value: string): string {
  // Profile release dates are documented as YYYY-MM-DD. `new Date(value)` would
  // parse that form as UTC midnight, which renders as the previous calendar day
  // for users west of UTC — parse the components as a local date instead.
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match) {
    const [, y, m, d] = match;
    const local = new Date(Number(y), Number(m) - 1, Number(d));
    if (!Number.isNaN(local.getTime())) {
      return local.toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
    }
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function ModelRow({
  model,
  providers,
  onDelete,
  onUpdate,
  onToggleEnabled,
  isTogglingEnabled,
}: {
  model: ModelWithProvider;
  providers: Provider[];
  onDelete: (id: string) => void;
  onUpdate: (id: string, data: UpdateModelRequest) => Promise<void>;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  isTogglingEnabled: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const profile = model.profile;

  return (
    <div className="border overflow-hidden">
      <div className="flex items-center justify-between p-3">
        <div className="flex items-center gap-3">
          <ModelIcon
            model={model}
            size="sm"
            showBackground={false}
            className="text-muted-foreground"
          />
          <div>
            <div className="font-medium flex items-center gap-2">
              {model.display_name}
              {model.enabled && (
                <Badge
                  variant="outline"
                  className="text-xs bg-green-50 text-green-700 border-green-200"
                >
                  Enabled
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
              {profile?.release_date && (
                <>
                  {" · "}
                  <span title={`Released ${profile.release_date}`}>
                    Released {formatReleaseDate(profile.release_date)}
                  </span>
                </>
              )}
              {profile && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setExpanded(!expanded)}
                  className="ml-2 h-6 px-1.5 align-baseline text-muted-foreground"
                  title={expanded ? "Collapse profile details" : "Expand profile details"}
                >
                  {expanded ? (
                    <ChevronUp className="h-3.5 w-3.5" />
                  ) : (
                    <ChevronDown className="h-3.5 w-3.5" />
                  )}
                </Button>
              )}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
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
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  role="img"
                  aria-label={model.healthy ? "Model healthy" : "Model not ready"}
                  className={
                    "inline-block h-2.5 w-2.5 rounded-full " +
                    (model.healthy ? "bg-green-500" : "bg-gray-300")
                  }
                />
              </TooltipTrigger>
              <TooltipContent>
                <p>
                  {model.healthy
                    ? "Model is configured and ready for use"
                    : "Provider is missing an API key or is disabled"}
                </p>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
          <Button
            variant={model.enabled ? "outline" : "default"}
            size="sm"
            onClick={() => onToggleEnabled(model.id, !model.enabled)}
            disabled={isTogglingEnabled}
            title={
              model.enabled
                ? "Disable model (hide from UI pickers)"
                : "Enable model (show in UI pickers)"
            }
          >
            {model.enabled ? (
              <>
                <ToggleRight className="h-4 w-4 mr-1" />
                Disable
              </>
            ) : (
              <>
                <ToggleLeft className="h-4 w-4 mr-1" />
                Enable
              </>
            )}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setEditOpen(true)}
            aria-label={`Edit ${model.display_name}`}
            title="Edit model"
          >
            <Pencil className="h-4 w-4" />
          </Button>
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

      {expanded && profile && (
        <div className="border-t bg-muted/30 p-4">
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm mb-3">
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
          {profile.supported_parameters && profile.supported_parameters.length > 0 && (
            <div className="border-t pt-3 mt-3">
              <div className="font-medium text-muted-foreground mb-2 flex items-center gap-1 text-sm">
                <Wrench className="h-3.5 w-3.5" />
                Supported Parameters
              </div>
              <div className="flex flex-wrap gap-1">
                {profile.supported_parameters.map((parameter) => (
                  <Badge key={parameter} variant="outline" className="text-xs font-normal">
                    {parameter}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          <div className="text-xs text-muted-foreground border-t pt-2 mt-2">
            Profile data from provider discovery and{" "}
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
      <EditModelDialog
        model={model}
        providers={providers}
        open={editOpen}
        onOpenChange={setEditOpen}
        onSubmit={onUpdate}
      />
    </div>
  );
}
