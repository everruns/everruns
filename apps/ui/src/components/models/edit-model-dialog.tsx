"use client";

import { useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ProviderIcon } from "@/components/providers/provider-icon";
import type { ModelWithProvider, Provider, UpdateModelRequest } from "@/lib/api/types";

export function EditModelDialog({
  model,
  providers,
  open,
  onOpenChange,
  onSubmit,
}: {
  model: ModelWithProvider;
  providers: Provider[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (modelId: string, data: UpdateModelRequest) => Promise<void>;
}) {
  const [providerId, setProviderId] = useState(model.provider_id);
  const [modelId, setModelId] = useState(model.model_id);
  const [displayName, setDisplayName] = useState(model.display_name);
  const [enabled, setEnabled] = useState(model.enabled);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setProviderId(model.provider_id);
    setModelId(model.model_id);
    setDisplayName(model.display_name);
    setEnabled(model.enabled);
  }, [model, open]);

  const selectedProvider = useMemo(
    () => providers.find((provider) => provider.id === providerId),
    [providerId, providers],
  );

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    try {
      await onSubmit(model.id, {
        provider_id: providerId,
        model_id: modelId.trim(),
        display_name: displayName.trim(),
        enabled,
      });
      onOpenChange(false);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit Model</DialogTitle>
          <DialogDescription>
            Update the model ID, display name, provider, or visibility.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor={`edit-model-provider-${model.id}`}>Provider</Label>
            <Select value={providerId} onValueChange={setProviderId}>
              <SelectTrigger id={`edit-model-provider-${model.id}`} className="w-full">
                <SelectValue placeholder="Select provider">
                  {selectedProvider?.name ?? model.provider_name}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {providers.map((provider) => (
                  <SelectItem key={provider.id} value={provider.id}>
                    <div className="flex items-center gap-2">
                      <ProviderIcon
                        providerType={provider.provider_type}
                        size="sm"
                        showBackground={false}
                      />
                      <span>{provider.name}</span>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor={`edit-model-id-${model.id}`}>Model ID</Label>
            <Input
              id={`edit-model-id-${model.id}`}
              value={modelId}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setModelId(e.target.value)}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor={`edit-model-display-name-${model.id}`}>Display Name</Label>
            <Input
              id={`edit-model-display-name-${model.id}`}
              value={displayName}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setDisplayName(e.target.value)}
              required
            />
          </div>
          <div className="flex items-center gap-2">
            <Checkbox
              id={`edit-model-enabled-${model.id}`}
              checked={enabled}
              onCheckedChange={(checked) => setEnabled(checked === true)}
            />
            <Label htmlFor={`edit-model-enabled-${model.id}`}>
              Enable model (visible in UI model pickers)
            </Label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={saving || !providerId || !modelId.trim() || !displayName.trim()}
            >
              {saving ? "Saving..." : "Save Changes"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
