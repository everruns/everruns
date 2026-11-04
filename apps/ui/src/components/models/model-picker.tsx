"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { Star } from "lucide-react";
import { ModelIcon } from "@/components/models/model-icon";
import { Combobox } from "@/components/ui/combobox";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ModelWithProvider, ModelService } from "@/lib/api/types";
import { matchesModelService } from "@/lib/model-capabilities";
import { useModels, useUpdateModel } from "@/hooks/use-providers";
import { useQueryClient } from "@tanstack/react-query";

export interface ModelPickerProps {
  service?: ModelService;
  /** Organization settings cannot reference personal provider credentials. */
  organizationDefault?: boolean;
  /** Operator Chat requires function tools; unknown custom profiles remain selectable. */
  requireTools?: boolean;
  additionalOptions?: { value: string; label: string }[];
  requiredPrimitives?: string[];
  /** Optional ID for the select trigger */
  id?: string;
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
  id,
  service = "chat",
  organizationDefault = false,
  requireTools = false,
  requiredPrimitives = [],
  additionalOptions = [],
  value,
  onChange,
  placeholder = "Select a model",
  disabled = false,
  showFavoriteToggle = false,
  className,
}: ModelPickerProps) {
  const ui = useUiTranslations();
  const { data: models = [], isLoading, error } = useModels();

  // Only show enabled models, sorted: favorites first, then by provider and name
  const sortedModels = [...models]
    .filter(
      (m) =>
        m.enabled &&
        !m.stale &&
        (!organizationDefault || m.provider_type !== "chatgpt") &&
        (!requireTools || m.profile?.tool_call !== false) &&
        m.healthy !== false &&
        matchesModelService(m, service) &&
        (service !== "decisions" || m.profile?.decisions?.calibrated === true) &&
        requiredPrimitives.every((primitive) =>
          m.profile?.decisions?.primitives.includes(primitive),
        ),
    )
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

  const selectedOption = additionalOptions.find((option) => option.value === value);
  const selectedModel = models.find((m) => m.id === value);

  return (
    <div className="space-y-2">
      <Combobox
        id={id}
        value={value || "none"}
        onValueChange={(next) => onChange(next === "none" ? "" : next)}
        ariaLabel={id ? undefined : ui(placeholder)}
        placeholder={ui(placeholder)}
        searchPlaceholder={ui("Search models")}
        valueLabel={
          selectedOption?.label ??
          (selectedModel
            ? `${selectedModel.display_name} (${selectedModel.provider_name})`
            : ui(placeholder))
        }
        disabled={disabled || isLoading}
        loading={isLoading}
        error={error ? ui("Could not load models. Try again.") : undefined}
        clearable={false}
        className={cn("w-full", className)}
        options={[
          { value: "none", label: ui(placeholder) },
          ...additionalOptions,
          ...sortedModels.map((model) => ({
            value: model.id,
            label: `${model.display_name} (${model.provider_name})`,
            icon: <ModelIcon model={model} size="sm" showBackground={false} />,
            description: model.is_favorite ? ui("Favorites") : undefined,
          })),
        ]}
      />
      {showFavoriteToggle && selectedModel && <FavoriteToggle model={selectedModel} size="sm" />}
      {!isLoading && !error && sortedModels.length === 0 && (
        <p className="text-xs text-muted-foreground">
          {ui("No configured")} {service} {ui("models.")}{" "}
          <Link href="/models" className="text-primary underline">
            {ui("Configure a model")}
          </Link>
          .
        </p>
      )}
      {!isLoading &&
        value &&
        value !== "none" &&
        !selectedOption &&
        !sortedModels.some((model) => model.id === value) && (
          <p role="alert" className="text-xs text-destructive">
            {ui("The configured model is unavailable or does not support")} {service}
            {ui(". Choose a model to repair this setting.")}
          </p>
        )}
    </div>
  );
}

// ============================================
// Standalone Favorite Toggle Component
// ============================================

interface FavoriteToggleProps {
  model: ModelWithProvider;
  size?: "sm" | "default";
}

/**
 * Standalone favorite toggle button for use in lists/tables
 */
export function FavoriteToggle({ model, size = "default" }: FavoriteToggleProps) {
  const ui = useUiTranslations();
  const queryClient = useQueryClient();
  const updateModel = useUpdateModel(model.id);

  const handleToggle = () => {
    updateModel.mutate(
      { is_favorite: !model.is_favorite },
      {
        onSuccess: () => {
          queryClient.invalidateQueries({ queryKey: ["models"] });
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
      title={model.is_favorite ? ui("Remove from favorites") : ui("Add to favorites")}
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

export function DecisionModelPicker(props: Omit<ModelPickerProps, "service">) {
  const ui = useUiTranslations();
  return (
    <ModelPicker
      {...props}
      service="decisions"
      placeholder={props.placeholder ?? ui("Select a decision model")}
    />
  );
}
