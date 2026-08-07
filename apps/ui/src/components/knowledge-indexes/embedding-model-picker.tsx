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

// Only advertise models whose registry capabilities explicitly include
// embeddings. Listing a generation-only fallback creates an index that cannot
// sync and hides the configuration problem until runtime.
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
  const candidates = enabled.filter(isEmbeddingModel);

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
            No embedding models available
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
