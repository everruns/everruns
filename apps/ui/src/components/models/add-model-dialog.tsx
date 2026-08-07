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
import { useCreateModel } from "@/hooks/use-providers";
import type { CreateModelRequest, Provider } from "@/lib/api/types";

type ModelType = "chat" | "embeddings";

export function AddModelDialog({
  providers,
  open,
  onOpenChange,
}: {
  providers: Provider[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [providerId, setProviderId] = useState("");
  const [modelId, setModelId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [modelType, setModelType] = useState<ModelType>("chat");
  const [enabled, setEnabled] = useState(true);

  const createModel = useCreateModel(providerId);
  const selectedProvider = providers.find((provider) => provider.id === providerId);

  const handleProviderChange = (nextProviderId: string) => {
    setProviderId(nextProviderId);
    const nextProvider = providers.find((provider) => provider.id === nextProviderId);
    if (nextProvider?.provider_type !== "openai") setModelType("chat");
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateModelRequest = {
      model_id: modelId,
      display_name: displayName,
      capabilities: modelType === "embeddings" ? ["embeddings"] : [],
      enabled,
    };
    await createModel.mutateAsync(data);
    onOpenChange(false);
    setProviderId("");
    setModelId("");
    setDisplayName("");
    setModelType("chat");
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
            <Select value={providerId} onValueChange={handleProviderChange}>
              <SelectTrigger id="provider" className="w-full">
                <SelectValue placeholder="Select provider">{selectedProvider?.name}</SelectValue>
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
            <Label htmlFor="model-type">Model type</Label>
            <Select value={modelType} onValueChange={(value) => setModelType(value as ModelType)}>
              <SelectTrigger id="model-type" className="w-full">
                <SelectValue>{modelType === "embeddings" ? "Embeddings" : "Chat"}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="chat">Chat</SelectItem>
                {selectedProvider?.provider_type === "openai" && (
                  <SelectItem value="embeddings">Embeddings</SelectItem>
                )}
              </SelectContent>
            </Select>
            {providerId && selectedProvider?.provider_type !== "openai" && (
              <p className="text-xs text-muted-foreground">
                This provider does not currently support embedding models.
              </p>
            )}
          </div>
          <div className="space-y-2">
            <Label htmlFor="model-id">Model ID</Label>
            <Input
              id="model-id"
              value={modelId}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setModelId(e.target.value)}
              placeholder={modelType === "embeddings" ? "text-embedding-3-small" : "gpt-5.2"}
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
