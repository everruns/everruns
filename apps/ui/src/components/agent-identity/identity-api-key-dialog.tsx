"use client";

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
import { useCreateIdentityApiKeyConnection } from "@/hooks/use-identity-connections";
import { AlertCircle } from "lucide-react";
import type { ConnectionProvider } from "@/lib/api/types";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { ProviderIcon } from "@/components/connections/provider-icon";

interface IdentityApiKeyDialogProps {
  identityId: string;
  provider: ConnectionProvider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function IdentityApiKeyDialog({
  identityId,
  provider,
  open,
  onOpenChange,
}: IdentityApiKeyDialogProps) {
  const [formValues, setFormValues] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const createConnection = useCreateIdentityApiKeyConnection(identityId);

  useEffect(() => {
    if (open) {
      setFormValues({});
      setError(null);
    }
  }, [open]);

  if (!provider?.form_schema) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    const apiKey = formValues["api_key"] ?? "";
    if (!apiKey.trim()) {
      setError("API key is required");
      return;
    }

    // Collect extra fields (everything except api_key)
    const extraFields: Record<string, string> = {};
    for (const [key, value] of Object.entries(formValues)) {
      if (key !== "api_key" && value.trim()) {
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
          <InlineStreamdownMessage className="text-sm text-muted-foreground mb-4 leading-relaxed">
            {provider.form_schema.instructions_markdown}
          </InlineStreamdownMessage>

          <div className="space-y-4 mb-4">
            {provider.form_schema.fields.map((field) => (
              <div key={field.name} className="space-y-2">
                <Label htmlFor={field.name}>{field.label}</Label>
                <Input
                  id={field.name}
                  type={field.field_type}
                  required={field.required}
                  placeholder={field.placeholder}
                  value={formValues[field.name] ?? ""}
                  onChange={(e) =>
                    setFormValues((prev) => ({
                      ...prev,
                      [field.name]: e.target.value,
                    }))
                  }
                  autoComplete="off"
                />
                {field.help_text && (
                  <p className="text-xs text-muted-foreground">{field.help_text}</p>
                )}
              </div>
            ))}
          </div>

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
