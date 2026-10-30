"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { useEffect } from "react";
import { Combobox } from "@/components/ui/combobox";
import { useModels } from "@/hooks/use-providers";
import { isEmbeddingModel } from "@/lib/model-capabilities";
import { cn } from "@/lib/utils";

interface EmbeddingModelPickerProps {
  id?: string;
  value: string;
  onChange: (modelId: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  "aria-invalid"?: boolean;
  "aria-describedby"?: string;
  onValidityChange?: (valid: boolean) => void;
}

export function EmbeddingModelPicker({
  id,
  value,
  onChange,
  placeholder = "Select an embedding model",
  disabled = false,
  className,
  "aria-invalid": ariaInvalid = false,
  "aria-describedby": ariaDescribedBy,
  onValidityChange,
}: EmbeddingModelPickerProps) {
  const ui = useUiTranslations();
  const { data: models = [], isLoading, error } = useModels();

  const candidates = models.filter((model) => model.enabled && isEmbeddingModel(model));
  const selectionValid = candidates.some((model) => model.id === value);

  useEffect(() => {
    onValidityChange?.(selectionValid);
  }, [onValidityChange, selectionValid]);

  const sorted = [...candidates].sort((a, b) => {
    if (a.provider_name !== b.provider_name) {
      return a.provider_name.localeCompare(b.provider_name);
    }
    return a.display_name.localeCompare(b.display_name);
  });

  const selectedModel = models.find((m) => m.id === value);
  const selectedIsInvalid = !!value && !isLoading && !selectionValid;

  return (
    <div className="space-y-2">
      <Combobox
        id={id}
        value={value}
        onValueChange={onChange}
        ariaLabel={ui("Embedding model")}
        aria-invalid={ariaInvalid}
        aria-describedby={ariaDescribedBy}
        valueLabel={
          selectedModel
            ? `${selectedModel.display_name} (${selectedModel.provider_name})`
            : undefined
        }
        placeholder={isLoading ? ui("Loading models...") : placeholder}
        searchPlaceholder={ui("Search models")}
        error={error ? ui("Could not load models. Try again.") : undefined}
        clearable={false}
        disabled={disabled || isLoading || sorted.length === 0}
        className={cn("w-full", className)}
        options={sorted.map((model) => ({
          value: model.id,
          label: `${model.display_name} (${model.provider_name})`,
        }))}
      />
      {!isLoading && !error && sorted.length === 0 && (
        <p className="text-xs text-muted-foreground">
          {ui("No embedding models are configured.")}{" "}
          <Link href="/models" className="text-primary underline underline-offset-4">
            {ui("Configure an embedding model")}
          </Link>
          .
        </p>
      )}
      {selectedIsInvalid && (
        <p className="text-xs text-destructive">
          {ui(
            "The configured model is unavailable or does not support embeddings. Choose an embedding model to repair this index.",
          )}
        </p>
      )}
    </div>
  );
}
