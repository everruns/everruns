"use client";

import Link from "next/link";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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

  const sorted = models
    .filter((m) => m.enabled && isEmbeddingModel(m))
    .sort((a, b) => {
      if (a.provider_name !== b.provider_name) {
        return a.provider_name.localeCompare(b.provider_name);
      }
      return a.display_name.localeCompare(b.display_name);
    });

  const selectedModel = models.find((m) => m.id === value);

  return (
    <div className="space-y-2">
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
      {!isLoading && sorted.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No embedding models are configured.{" "}
          <Link href="/models" className="text-primary underline underline-offset-4">
            Configure an embedding model
          </Link>
          .
        </p>
      )}
    </div>
  );
}
