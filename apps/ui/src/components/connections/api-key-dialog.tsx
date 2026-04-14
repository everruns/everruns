"use client";

/**
 * Shared ApiKeyDialog for entering API key connections.
 * Used by both the Settings > Connections page and inline chat connection prompts.
 */

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useCreateApiKeyConnection } from "@/hooks/use-user-connections";
import { AlertCircle } from "lucide-react";
import type { ConnectionProvider } from "@/lib/api/types";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import {
  createConnectionFormSchema,
  getFieldErrors,
  type FieldErrors,
} from "@/lib/form-validation";
import { ProviderIcon } from "./provider-icon";

interface ApiKeyDialogProps {
  provider: ConnectionProvider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called after a successful connection is created */
  onConnected?: () => void;
}

export function ApiKeyDialog({ provider, open, onOpenChange, onConnected }: ApiKeyDialogProps) {
  const [formValues, setFormValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});
  const createConnection = useCreateApiKeyConnection();

  // Reset form when dialog opens/closes
  useEffect(() => {
    if (open) {
      setFormValues({});
      setError(null);
      setFieldErrors({});
    }
  }, [open]);

  const formSchema = provider?.form_schema;
  if (!provider || !formSchema) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setFieldErrors({});

    const parsed = createConnectionFormSchema(formSchema.fields).safeParse(formValues);
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    const apiKey = parsed.data["api_key"] as string;

    // Collect extra fields (everything except api_key)
    const extraFields: Record<string, string> = {};
    for (const [key, value] of Object.entries(parsed.data)) {
      if (key !== "api_key" && typeof value === "string" && value.trim()) {
        extraFields[key] = value;
      }
    }

    try {
      await createConnection.mutateAsync({
        provider: provider.provider_id,
        apiKey,
        extraFields: Object.keys(extraFields).length > 0 ? extraFields : undefined,
      });
      onOpenChange(false);
      onConnected?.();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to save connection";
      const apiError = (err as { response?: { data?: string } })?.response?.data;
      setError(typeof apiError === "string" ? apiError : message);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ProviderIcon iconName={provider.icon} className="h-5 w-5" />
            Connect {provider.display_name}
          </DialogTitle>
          <DialogDescription>{provider.description}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit}>
          {/* Instructions */}
          <InlineStreamdownMessage className="text-sm text-muted-foreground mb-4 leading-relaxed">
            {formSchema.instructions_markdown}
          </InlineStreamdownMessage>

          {/* Form fields */}
          <div className="space-y-4 mb-4">
            {formSchema.fields.map((field) => (
              <div key={field.name} className="space-y-2">
                <Label htmlFor={field.name}>{field.label}</Label>
                <Input
                  id={field.name}
                  type={field.field_type}
                  required={field.required}
                  placeholder={field.placeholder}
                  value={formValues[field.name] ?? ""}
                  aria-invalid={!!fieldErrors[field.name]}
                  onChange={(e) => {
                    setFormValues((prev) => ({
                      ...prev,
                      [field.name]: e.target.value,
                    }));
                    setFieldErrors((prev) => {
                      if (!prev[field.name]) return prev;
                      return { ...prev, [field.name]: undefined };
                    });
                  }}
                  autoComplete="off"
                />
                {fieldErrors[field.name] && (
                  <p className="text-xs text-destructive">{fieldErrors[field.name]}</p>
                )}
                {field.help_text && (
                  <p className="text-xs text-muted-foreground">{field.help_text}</p>
                )}
              </div>
            ))}
          </div>

          {/* Error */}
          {error && (
            <div className="flex items-center gap-2 text-destructive text-sm mb-4">
              <AlertCircle className="h-4 w-4 flex-shrink-0" />
              {error}
            </div>
          )}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createConnection.isPending}>
              {createConnection.isPending ? "Validating..." : "Connect"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
