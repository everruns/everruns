"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useCreateLlmProvider,
  useUpdateLlmProvider,
  useCreateLlmModel,
} from "@/hooks/use-llm-providers";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import type {
  LlmProvider,
  LlmProviderType,
  CreateLlmProviderRequest,
  CreateLlmModelRequest,
} from "@/lib/api/types";

const PROVIDER_TYPES: { value: LlmProviderType; label: string }[] = [
  { value: "openai", label: "OpenAI (Responses API)" },
  { value: "openai_completions", label: "OpenAI (Completions API)" },
  { value: "anthropic", label: "Anthropic" },
  { value: "gemini", label: "Google Gemini" },
];

// Get API key placeholder based on provider type
function getApiKeyPlaceholder(providerType: LlmProviderType): string {
  switch (providerType) {
    case "openai":
    case "openai_completions":
      return "sk-...";
    case "anthropic":
      return "sk-ant-api03-...";
    default:
      return "your-api-key";
  }
}

export function AddProviderDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [providerType, setProviderType] = useState<LlmProviderType>("openai");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");

  const createProvider = useCreateLlmProvider();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateLlmProviderRequest = {
      name,
      provider_type: providerType,
      base_url: baseUrl || undefined,
      api_key: apiKey || undefined,
    };
    await createProvider.mutateAsync(data);
    onOpenChange(false);
    setName("");
    setProviderType("openai");
    setBaseUrl("");
    setApiKey("");
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add LLM Provider</DialogTitle>
          <DialogDescription>
            Configure a new LLM provider for your agents to use.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="name">Name</Label>
            <Input
              id="name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="My OpenAI Provider"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="provider-type">Provider Type</Label>
            <Select
              value={providerType}
              onValueChange={(v) => setProviderType(v as LlmProviderType)}
            >
              <SelectTrigger className="w-full">
                <div className="flex items-center gap-2">
                  <ProviderIcon providerType={providerType} size="sm" showBackground={false} />
                  <span>{getProviderLabel(providerType)}</span>
                </div>
              </SelectTrigger>
              <SelectContent>
                {PROVIDER_TYPES.map((type) => (
                  <SelectItem key={type.value} value={type.value}>
                    <div className="flex items-center gap-2">
                      <ProviderIcon providerType={type.value} size="sm" showBackground={false} />
                      <span>{type.label}</span>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="base-url">Base URL (optional)</Label>
            <Input
              id="base-url"
              value={baseUrl}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setBaseUrl(e.target.value)}
              placeholder="https://api.openai.com/v1"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="api-key">API Key (optional)</Label>
            <Input
              id="api-key"
              type="password"
              value={apiKey}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setApiKey(e.target.value)}
              placeholder={getApiKeyPlaceholder(providerType)}
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createProvider.isPending || !name}>
              {createProvider.isPending ? "Creating..." : "Create Provider"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SetApiKeyDialog({
  provider,
  open,
  onOpenChange,
}: {
  provider: LlmProvider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const updateProvider = useUpdateLlmProvider(provider?.id || "");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!provider) return;
    await updateProvider.mutateAsync({ api_key: apiKey });
    onOpenChange(false);
    setApiKey("");
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{provider?.api_key_set ? "Update" : "Set"} API Key</DialogTitle>
          <DialogDescription>
            {provider?.api_key_set
              ? "Enter a new API key to replace the existing one."
              : "Enter the API key for this provider."}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="new-api-key">API Key</Label>
            <Input
              id="new-api-key"
              type="password"
              value={apiKey}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setApiKey(e.target.value)}
              placeholder={provider ? getApiKeyPlaceholder(provider.provider_type) : "your-api-key"}
              required
            />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={updateProvider.isPending || !apiKey}>
              {updateProvider.isPending ? "Saving..." : "Save API Key"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

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
              <SelectTrigger className="w-full">
                <span className={!providerId ? "text-muted-foreground" : ""}>
                  {providerId
                    ? providers.find((p) => p.id === providerId)?.name
                    : "Select provider"}
                </span>
              </SelectTrigger>
              <SelectContent>
                {providers.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.name}
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
            <input
              type="checkbox"
              id="model-enabled"
              checked={enabled}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setEnabled(e.target.checked)}
              className="h-4 w-4"
            />
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
