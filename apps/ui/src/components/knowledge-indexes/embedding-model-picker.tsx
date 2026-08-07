"use client";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useModels } from "@/hooks/use-providers";
import type { ModelWithProvider } from "@/lib/api/types";
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
}

// A model is treated as an embeddings model when its capabilities advertise
// "embeddings". The backend model registry does not reliably tag embedding
// models through the list API today (model sync leaves capabilities empty for
// many providers), so we fall back to listing all enabled models when no model
// advertises the capability. The server still enforces that the chosen model
// resolves to an Embeddings driver, so an invalid pick is rejected on save.
export function isEmbeddingModel(model: ModelWithProvider): boolean {
  return model.capabilities.some((cap) => cap.toLowerCase() === "embeddings");
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
}: EmbeddingModelPickerProps) {
  const { data: models = [], isLoading } = useModels();

  const enabled = models.filter((m) => m.enabled);
  const embeddingModels = enabled.filter(isEmbeddingModel);
  // Fall back to all enabled models when the registry exposes no embeddings
  // capability so the picker is never empty; the server validates the choice.
  const candidates = embeddingModels.length > 0 ? embeddingModels : enabled;

  const sorted = [...candidates].sort((a, b) => {
    if (a.provider_name !== b.provider_name) {
      return a.provider_name.localeCompare(b.provider_name);
    }
    return a.display_name.localeCompare(b.display_name);
  });

  const selectedModel = models.find((m) => m.id === value);

  return (
    <Select value={value} onValueChange={onChange} disabled={disabled || isLoading}>
      <SelectTrigger
        id={id}
        className={cn("w-full", className)}
        aria-label="Embedding model"
        aria-invalid={ariaInvalid}
        aria-describedby={ariaDescribedBy}
      >
        <SelectValue placeholder={isLoading ? "Loading models..." : placeholder}>
          {selectedModel
            ? `${selectedModel.display_name} (${selectedModel.provider_name})`
            : undefined}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {sorted.length === 0 && (
          <SelectItem value="__none__" disabled>
            No models available
          </SelectItem>
        )}
        {sorted.map((model) => (
          <SelectItem key={model.id} value={model.id}>
            {model.display_name} ({model.provider_name})
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
