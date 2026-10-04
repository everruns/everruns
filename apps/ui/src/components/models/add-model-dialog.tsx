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
import { useCreateModel, useProvidersConfig } from "@/hooks/use-providers";
import type { CreateModelRequest, Provider } from "@/lib/api/types";

type ModelType = "chat" | "embeddings" | "decisions";
import { ModelProfileSelect } from "./model-profile-select";

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

  const [profileKey, setProfileKey] = useState<string | undefined>();
  const { data: providerConfig } = useProvidersConfig();
  const createModel = useCreateModel(providerId);
  const selectedProvider = providers.find((provider) => provider.id === providerId);

  const services =
    providerConfig?.drivers?.find((driver) => driver.driver === selectedProvider?.provider_type)
      ?.services ??
    (selectedProvider?.provider_type === "openai"
      ? ["chat", "embeddings"]
      : selectedProvider?.provider_type === "typesafe"
        ? ["decisions"]
        : selectedProvider?.provider_type === "openrouter"
          ? ["chat", "decisions"]
          : ["chat"]);

  const handleProviderChange = (nextProviderId: string) => {
    setProviderId(nextProviderId);
    const nextProvider = providers.find((provider) => provider.id === nextProviderId);
    setModelType(nextProvider?.provider_type === "typesafe" ? "decisions" : "chat");
    setProfileKey(undefined);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateModelRequest = {
      model_id: modelId,
      display_name: displayName,
      capabilities: [modelType],
      service: modelType,
      profile_key: profileKey,
      enabled,
    };
    await createModel.mutateAsync(data);
    onOpenChange(false);
    setProviderId("");
    setModelId("");
    setDisplayName("");
    setModelType("chat");
    setProfileKey(undefined);
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
            <Select
              value={modelType}
              onValueChange={(value) => {
                setModelType(value as ModelType);
                setProfileKey(undefined);
              }}
            >
              <SelectTrigger id="model-type" className="w-full">
                <SelectValue>
                  {modelType === "embeddings"
                    ? "Embeddings"
                    : modelType === "decisions"
                      ? "Decisions"
                      : "Chat"}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {services
                  .filter((service) => ["chat", "embeddings", "decisions"].includes(service))
                  .map((service) => (
                    <SelectItem key={service} value={service}>
                      {service === "chat"
                        ? "Chat"
                        : service === "decisions"
                          ? "Decisions"
                          : "Embeddings"}
                    </SelectItem>
                  ))}
              </SelectContent>
            </Select>
          </div>
          {providerId && (
            <ModelProfileSelect
              providerId={providerId}
              service={modelType}
              value={profileKey}
              onChange={(profile) => {
                setProfileKey(profile?.key);
                if (profile) {
                  const canonical = profile.key.split("/").slice(1).join("/");
                  setModelId(
                    modelType === "decisions" && selectedProvider?.provider_type === "openrouter"
                      ? `typesafe/${canonical === "jev-1.13.0" ? "jev-1.13" : canonical}`
                      : canonical,
                  );
                  setDisplayName(profile.profile.name);
                }
              }}
            />
          )}
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
