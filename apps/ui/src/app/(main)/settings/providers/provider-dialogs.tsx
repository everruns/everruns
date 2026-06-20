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
import { useCreateProvider, useUpdateProvider } from "@/hooks/use-providers";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import type { Provider, DriverId, CreateProviderRequest } from "@/lib/api/types";

const PROVIDER_TYPES: { value: DriverId; label: string }[] = [
  { value: "openai", label: "OpenAI (Responses API)" },
  { value: "openrouter", label: "OpenRouter" },
  { value: "azure_openai", label: "Azure OpenAI" },
  { value: "openai_completions", label: "OpenAI (Completions API)" },
  { value: "anthropic", label: "Anthropic" },
  { value: "gemini", label: "Google Gemini" },
  { value: "bedrock", label: "AWS Bedrock" },
  { value: "mai", label: "Microsoft MAI" },
  { value: "fireworks", label: "Fireworks AI" },
];

// Get API key placeholder based on provider type
function getApiKeyPlaceholder(providerType: DriverId): string {
  switch (providerType) {
    case "openai":
    case "openai_completions":
      return "sk-...";
    case "openrouter":
      return "sk-or-...";
    case "azure_openai":
      return "Azure OpenAI resource key";
    case "anthropic":
      return "sk-ant-api03-...";
    case "bedrock":
      return '{"access_key_id":"...","secret_access_key":"...","region":"us-east-1"}';
    case "mai":
      return 'Foundry key, or {"tenant_id":"...","client_id":"...","client_secret":"..."}';
    case "fireworks":
      return "fw_...";
    default:
      return "your-api-key";
  }
}

function getBaseUrlPlaceholder(providerType: DriverId): string {
  switch (providerType) {
    case "azure_openai":
      return "https://your-resource.openai.azure.com/openai/v1";
    case "openai":
      return "https://api.openai.com/v1";
    case "openrouter":
      return "https://openrouter.ai/api/v1";
    case "openai_completions":
      return "https://api.openai.com/v1/chat/completions";
    case "anthropic":
      return "https://api.anthropic.com/v1/messages";
    case "gemini":
      return "https://generativelanguage.googleapis.com";
    case "mai":
      return "https://your-resource.services.ai.azure.com";
    case "fireworks":
      return "https://api.fireworks.ai/inference/v1";
    default:
      return "https://api.example.com";
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
  const [providerType, setProviderType] = useState<DriverId>("openai");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");

  const createProvider = useCreateProvider();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateProviderRequest = {
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
            <Select value={providerType} onValueChange={(v) => setProviderType(v as DriverId)}>
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
              placeholder={getBaseUrlPlaceholder(providerType)}
              required={providerType === "azure_openai"}
            />
            {providerType === "azure_openai" ? (
              <p className="text-sm text-muted-foreground">
                Use your Azure OpenAI `.../openai/v1` endpoint on `openai.azure.com` or
                `services.ai.azure.com`.
              </p>
            ) : null}
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
            <Button
              type="submit"
              disabled={
                createProvider.isPending ||
                !name ||
                (providerType === "azure_openai" && !baseUrl.trim())
              }
            >
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
  provider: Provider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const updateProvider = useUpdateProvider(provider?.id || "");

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
