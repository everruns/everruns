"use client";

import { Star } from "lucide-react";
import { ProviderIcon } from "@/components/providers/provider-icon";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { LlmModelWithProvider } from "@/lib/api/types";
import { useLlmModels, useUpdateLlmModel } from "@/hooks/use-llm-providers";
import { useQueryClient } from "@tanstack/react-query";

interface ModelPickerProps {
  /** Current selected model ID (empty string or "none" for no selection) */
  value: string;
  /** Called when model selection changes */
  onChange: (modelId: string) => void;
  /** Text to show when no model is selected */
  placeholder?: string;
  /** Whether the picker is disabled */
  disabled?: boolean;
  /** Show favorite toggle buttons */
  showFavoriteToggle?: boolean;
  /** Additional class name for the trigger */
  className?: string;
}

/**
 * Model picker component that shows available LLM models.
 * - Favorites are shown first
 * - Disabled/unconfigured models are hidden
 * - Optionally shows favorite toggle buttons
 */
export function ModelPicker({
  value,
  onChange,
  placeholder = "Select a model",
  disabled = false,
  showFavoriteToggle = false,
  className,
}: ModelPickerProps) {
  const { data: models = [], isLoading } = useLlmModels();

  // Only show installed models, sorted: favorites first, then by provider and name
  const sortedModels = [...models]
    .filter((m) => m.installed)
    .sort((a, b) => {
      // Favorites first
      if (a.is_favorite !== b.is_favorite) {
        return a.is_favorite ? -1 : 1;
      }
      // Then by provider name
      if (a.provider_name !== b.provider_name) {
        return a.provider_name.localeCompare(b.provider_name);
      }
      // Then by display name
      return a.display_name.localeCompare(b.display_name);
    });

  const selectedModel = models.find((m) => m.id === value);

  return (
    <Select
      value={value || "none"}
      onValueChange={(val) => onChange(val === "none" ? "" : val)}
      disabled={disabled || isLoading}
    >
      <SelectTrigger className={cn("w-full", className)}>
        <SelectValue>
          {(() => {
            if (isLoading) return "Loading...";
            if (!selectedModel) return placeholder;
            return (
              <div className="flex items-center gap-2">
                <ProviderIcon
                  providerType={selectedModel.provider_type}
                  size="sm"
                  showBackground={false}
                />
                <span>
                  {selectedModel.display_name} ({selectedModel.provider_name})
                </span>
                {selectedModel.is_favorite && (
                  <Star className="w-3 h-3 fill-yellow-400 text-yellow-400" />
                )}
              </div>
            );
          })()}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="none">{placeholder}</SelectItem>
        {sortedModels.map((model) => (
          <ModelSelectItem key={model.id} model={model} showFavoriteToggle={showFavoriteToggle} />
        ))}
      </SelectContent>
    </Select>
  );
}

interface ModelSelectItemProps {
  model: LlmModelWithProvider;
  showFavoriteToggle: boolean;
}

function ModelSelectItem({ model, showFavoriteToggle }: ModelSelectItemProps) {
  const queryClient = useQueryClient();
  const updateModel = useUpdateLlmModel(model.id);

  const handleToggleFavorite = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();

    updateModel.mutate(
      { is_favorite: !model.is_favorite },
      {
        onSuccess: () => {
          queryClient.invalidateQueries({ queryKey: ["llm-models"] });
        },
      },
    );
  };

  return (
    <SelectItem value={model.id}>
      <div className="flex items-center gap-2 w-full">
        <ProviderIcon providerType={model.provider_type} size="sm" showBackground={false} />
        <span className="flex-1">
          {model.display_name} ({model.provider_name})
        </span>
        {model.is_favorite && <Star className="w-3 h-3 fill-yellow-400 text-yellow-400" />}
        {showFavoriteToggle && (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 w-6 p-0"
            onClick={handleToggleFavorite}
            disabled={updateModel.isPending}
          >
            <Star
              className={cn(
                "w-4 h-4",
                model.is_favorite ? "fill-yellow-400 text-yellow-400" : "text-muted-foreground",
              )}
            />
          </Button>
        )}
      </div>
    </SelectItem>
  );
}

// ============================================
// Standalone Favorite Toggle Component
// ============================================

interface FavoriteToggleProps {
  model: LlmModelWithProvider;
  size?: "sm" | "default";
}

/**
 * Standalone favorite toggle button for use in lists/tables
 */
export function FavoriteToggle({ model, size = "default" }: FavoriteToggleProps) {
  const queryClient = useQueryClient();
  const updateModel = useUpdateLlmModel(model.id);

  const handleToggle = () => {
    updateModel.mutate(
      { is_favorite: !model.is_favorite },
      {
        onSuccess: () => {
          queryClient.invalidateQueries({ queryKey: ["llm-models"] });
        },
      },
    );
  };

  const iconSize = size === "sm" ? "w-4 h-4" : "w-5 h-5";
  const buttonSize = size === "sm" ? "h-6 w-6" : "h-8 w-8";

  return (
    <Button
      variant="ghost"
      size="sm"
      className={cn(buttonSize, "p-0")}
      onClick={handleToggle}
      disabled={updateModel.isPending}
      title={model.is_favorite ? "Remove from favorites" : "Add to favorites"}
    >
      <Star
        className={cn(
          iconSize,
          model.is_favorite
            ? "fill-yellow-400 text-yellow-400"
            : "text-muted-foreground hover:text-yellow-400",
        )}
      />
    </Button>
  );
}
