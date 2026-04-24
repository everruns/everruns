"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useCreateLlmModel } from "@/hooks/use-llm-providers";
import type { CreateLlmModelRequest, LlmProvider } from "@/lib/api/types";

export function AddModelDialog({
  providers,
  open,
  onOpenChange,
}: {
  providers: LlmProvider[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [providerId, setProviderId] = useState("");
  const [modelId, setModelId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [enabled, setEnabled] = useState(true);

  const createModel = useCreateLlmModel(providerId);
  const selectedProviderName = providers.find((provider) => provider.id === providerId)?.name;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateLlmModelRequest = {
      model_id: modelId,
      display_name: displayName,
      enabled,
    };
    await createModel.mutateAsync(data);
    onOpenChange(false);
    setProviderId("");
    setModelId("");
    setDisplayName("");
    setEnabled(true);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Model</DialogTitle>
          <DialogDescription>Add a new model to an existing provider.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="provider">Provider</Label>
            <Select value={providerId} onValueChange={setProviderId}>
              <SelectTrigger id="provider" className="w-full">
                <SelectValue placeholder="Select provider">{selectedProviderName}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                {providers.map((provider) => (
                  <SelectItem key={provider.id} value={provider.id}>
                    {provider.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="model-id">Model ID</Label>
            <Input
              id="model-id"
              value={modelId}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setModelId(e.target.value)}
              placeholder="gpt-5.2"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="display-name">Display Name</Label>
            <Input
              id="display-name"
              value={displayName}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setDisplayName(e.target.value)}
              placeholder="GPT-4o"
              required
            />
          </div>
          <div className="flex items-center gap-2">
            <Checkbox id="model-enabled" checked={enabled} onCheckedChange={setEnabled} />
            <Label htmlFor="model-enabled">Enable model (visible in UI model pickers)</Label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={createModel.isPending || !providerId || !modelId || !displayName}
            >
              {createModel.isPending ? "Creating..." : "Create Model"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
