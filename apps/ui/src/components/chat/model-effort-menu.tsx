/**
 * Decisions:
 * - Collapse the model + reasoning-effort controls into one composer chip that opens a
 *   settings-style menu with a submenu per control, so the composer footer stays compact.
 * - The trigger summarizes the active choices ("<model> <effort>") the way a status chip does;
 *   the submenus own the actual list of options and mark the current one with a check.
 * - Recents live at the top of the Model submenu. They are passed in already resolved against the
 *   live model list, so entries that no longer exist never render here.
 */
"use client";

import { Check, ChevronDown, RotateCcw, Zap } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { cn } from "@/lib/utils";
import type { Model, ReasoningEffortConfig } from "@/lib/api/types";
import { useLocale } from "@/providers/locale-provider";

interface ModelEffortMenuProps {
  models: Model[];
  recentModels: Model[];
  selectedModelId: string;
  onModelChange: (value: string) => void;
  modelTriggerLabel: string;
  defaultModelOptionLabel: string;
  supportsReasoning: boolean;
  reasoningEffort: string;
  reasoningEffortConfig?: ReasoningEffortConfig;
  defaultEffortName: string;
  getReasoningEffortName: (value: string) => string;
  onReasoningEffortChange: (value: string) => void;
}

/** A menu row that marks the active option with a trailing check, like the screenshot. */
function OptionItem({
  label,
  selected,
  onSelect,
}: {
  label: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <DropdownMenuItem onClick={onSelect} className="pr-2">
      <span className="flex-1 truncate">{label}</span>
      {selected && <Check className="ml-auto size-4 text-foreground" />}
    </DropdownMenuItem>
  );
}

export function ModelEffortMenu({
  models,
  recentModels,
  selectedModelId,
  onModelChange,
  modelTriggerLabel,
  defaultModelOptionLabel,
  supportsReasoning,
  reasoningEffort,
  reasoningEffortConfig,
  defaultEffortName,
  getReasoningEffortName,
  onReasoningEffortChange,
}: ModelEffortMenuProps) {
  const { t } = useLocale();

  const enabledModels = models.filter((model) => model.enabled);
  const recentIds = new Set(recentModels.map((model) => model.id));

  // Effective effort name for the trigger: the explicit override, or the model's default.
  const effortSummary = reasoningEffort
    ? getReasoningEffortName(reasoningEffort)
    : defaultEffortName;

  const canReset = Boolean(selectedModelId) || Boolean(reasoningEffort);
  const handleReset = () => {
    onModelChange("");
    if (supportsReasoning) onReasoningEffortChange("");
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(
          chatSurfaceStyles.composerControlChip,
          "cursor-pointer gap-1.5 outline-hidden hover:bg-muted/40 focus-visible:ring-1 focus-visible:ring-primary/20 data-[popup-open]:bg-muted/40 [&_svg]:icon-sharp",
        )}
        aria-label={t("model")}
      >
        <Zap className="size-3.5 text-muted-foreground" />
        <span className="max-w-[140px] truncate font-medium">{modelTriggerLabel}</span>
        {supportsReasoning && <span className="text-muted-foreground">{effortSummary}</span>}
        <ChevronDown className="size-3.5 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="start" sideOffset={6}>
        <DropdownMenuContent className="min-w-[200px]">
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>
              <span>{t("model")}</span>
              <span className="ml-auto max-w-[120px] truncate text-muted-foreground">
                {modelTriggerLabel}
              </span>
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent className="max-h-[320px] min-w-[200px] overflow-y-auto">
              <DropdownMenuGroup>
                <DropdownMenuLabel>{t("model")}</DropdownMenuLabel>
                <OptionItem
                  label={defaultModelOptionLabel}
                  selected={selectedModelId === ""}
                  onSelect={() => onModelChange("")}
                />
              </DropdownMenuGroup>
              {recentModels.length > 0 && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuGroup>
                    <DropdownMenuLabel>{t("recent")}</DropdownMenuLabel>
                    {recentModels.map((model) => (
                      <OptionItem
                        key={`recent-${model.id}`}
                        label={model.display_name}
                        selected={selectedModelId === model.id}
                        onSelect={() => onModelChange(model.id)}
                      />
                    ))}
                  </DropdownMenuGroup>
                </>
              )}
              {enabledModels.length > 0 && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuGroup>
                    {enabledModels.map((model) => (
                      <OptionItem
                        key={model.id}
                        label={model.display_name}
                        selected={selectedModelId === model.id && !recentIds.has(model.id)}
                        onSelect={() => onModelChange(model.id)}
                      />
                    ))}
                  </DropdownMenuGroup>
                </>
              )}
            </DropdownMenuSubContent>
          </DropdownMenuSub>

          {supportsReasoning && reasoningEffortConfig && (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <span>{t("reasoning")}</span>
                <span className="ml-auto text-muted-foreground">{effortSummary}</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="min-w-[180px]">
                <DropdownMenuGroup>
                  <DropdownMenuLabel>{t("reasoning")}</DropdownMenuLabel>
                  <OptionItem
                    label={t("default_with_value", { value: defaultEffortName })}
                    selected={reasoningEffort === ""}
                    onSelect={() => onReasoningEffortChange("")}
                  />
                  {reasoningEffortConfig.values.map((effort) => (
                    <OptionItem
                      key={effort.value}
                      label={effort.name}
                      selected={reasoningEffort === effort.value}
                      onSelect={() => onReasoningEffortChange(effort.value)}
                    />
                  ))}
                </DropdownMenuGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          )}

          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={handleReset} disabled={!canReset}>
            <RotateCcw className="size-4" />
            <span>{t("reset_to_default")}</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
