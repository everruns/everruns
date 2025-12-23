"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
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
import {
  useLlmProviders,
  useLlmModels,
  useCreateLlmProvider,
  useUpdateLlmProvider,
  useDeleteLlmProvider,
  useCreateLlmModel,
  useDeleteLlmModel,
} from "@/hooks/use-llm-providers";
import {
  Plus,
  Server,
  Trash2,
  Key,
  Star,
  Cpu,
} from "lucide-react";
import type {
  LlmProvider,
  LlmModelWithProvider,
  LlmProviderType,
  CreateLlmProviderRequest,
  CreateLlmModelRequest,
} from "@/lib/api/types";

const PROVIDER_TYPES: { value: LlmProviderType; label: string }[] = [
  { value: "openai", label: "OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "azure_openai", label: "Azure OpenAI" },
];

// Get API key placeholder based on provider type
function getApiKeyPlaceholder(providerType: LlmProviderType): string {
  switch (providerType) {
    case "openai":
      return "sk-...";
    case "anthropic":
      return "sk-ant-api03-...";
    case "azure_openai":
      return "your-azure-api-key";
    default:
      return "your-api-key";
  }
}

function ProviderCard({
  provider,
  onDelete,
  onSetApiKey,
}: {
  provider: LlmProvider;
  onDelete: (id: string) => void;
  onSetApiKey: (provider: LlmProvider) => void;
}) {
  const providerLabel =
    PROVIDER_TYPES.find((t) => t.value === provider.provider_type)?.label ||
    provider.provider_type;

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <div className="p-2 bg-primary/10 rounded-lg">
            <Server className="h-5 w-5 text-primary" />
          </div>
          <div>
            <CardTitle className="text-lg flex items-center gap-2">
              {provider.name}
              {provider.is_default && (
                <Star className="h-4 w-4 text-yellow-500 fill-yellow-500" />
              )}
            </CardTitle>
            <CardDescription className="text-sm">{providerLabel}</CardDescription>
          </div>
        </div>
        <Badge
          variant="outline"
          className={
            provider.status === "active"
              ? "bg-green-100 text-green-800"
              : "bg-gray-100 text-gray-800"
          }
        >
          {provider.status}
        </Badge>
      </CardHeader>
      <CardContent>
        <div className="space-y-2 text-sm">
          {provider.base_url && (
            <p className="text-muted-foreground truncate">
              URL: {provider.base_url}
            </p>
          )}
          <div className="flex items-center gap-2">
            <Key className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground">
              API Key: {provider.api_key_set ? "Configured" : "Not set"}
            </span>
          </div>
        </div>
        <div className="flex items-center justify-end gap-2 mt-4">
          <Button variant="outline" size="sm" onClick={() => onSetApiKey(provider)}>
            <Key className="h-4 w-4 mr-1" />
            {provider.api_key_set ? "Update Key" : "Set Key"}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive"
            onClick={() => onDelete(provider.id)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function ModelRow({
  model,
  onDelete,
}: {
  model: LlmModelWithProvider;
  onDelete: (id: string) => void;
}) {
  return (
    <div className="flex items-center justify-between p-3 border rounded-lg">
      <div className="flex items-center gap-3">
        <Cpu className="h-5 w-5 text-muted-foreground" />
        <div>
          <div className="font-medium flex items-center gap-2">
            {model.display_name}
            {model.is_default && (
              <Star className="h-3 w-3 text-yellow-500 fill-yellow-500" />
            )}
          </div>
          <div className="text-sm text-muted-foreground">
            {model.model_id} - {model.provider_name}
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2">
        {model.capabilities.length > 0 && (
          <div className="flex gap-1">
            {model.capabilities.slice(0, 2).map((cap) => (
              <Badge key={cap} variant="secondary" className="text-xs">
                {cap}
              </Badge>
            ))}
            {model.capabilities.length > 2 && (
              <Badge variant="secondary" className="text-xs">
                +{model.capabilities.length - 2}
              </Badge>
            )}
          </div>
        )}
        <Badge
          variant="outline"
          className={
            model.status === "active"
              ? "bg-green-100 text-green-800"
              : "bg-gray-100 text-gray-800"
          }
        >
          {model.status}
        </Badge>
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          onClick={() => onDelete(model.id)}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function AddProviderDialog({
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
  const [isDefault, setIsDefault] = useState(false);

  const createProvider = useCreateLlmProvider();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateLlmProviderRequest = {
      name,
      provider_type: providerType,
      base_url: baseUrl || undefined,
      api_key: apiKey || undefined,
      is_default: isDefault,
    };
    await createProvider.mutateAsync(data);
    onOpenChange(false);
    setName("");
    setProviderType("openai");
    setBaseUrl("");
    setApiKey("");
    setIsDefault(false);
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
            <Select value={providerType} onValueChange={(v) => setProviderType(v as LlmProviderType)}>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Select provider type" />
              </SelectTrigger>
              <SelectContent>
                {PROVIDER_TYPES.map((type) => (
                  <SelectItem key={type.value} value={type.value}>
                    {type.label}
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
          <div className="flex items-center gap-2">
            <input
              type="checkbox"
              id="is-default"
              checked={isDefault}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setIsDefault(e.target.checked)}
              className="h-4 w-4"
            />
            <Label htmlFor="is-default">Set as default provider</Label>
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

function SetApiKeyDialog({
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
          <DialogTitle>
            {provider?.api_key_set ? "Update" : "Set"} API Key
          </DialogTitle>
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

function AddModelDialog({
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
  const [contextWindow, setContextWindow] = useState("");
  const [isDefault, setIsDefault] = useState(false);

  const createModel = useCreateLlmModel(providerId);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateLlmModelRequest = {
      model_id: modelId,
      display_name: displayName,
      context_window: contextWindow ? parseInt(contextWindow) : undefined,
      is_default: isDefault,
    };
    await createModel.mutateAsync(data);
    onOpenChange(false);
    setProviderId("");
    setModelId("");
    setDisplayName("");
    setContextWindow("");
    setIsDefault(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Model</DialogTitle>
          <DialogDescription>
            Add a new model to an existing provider.
          </DialogDescription>
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
          <div className="space-y-2">
            <Label htmlFor="context-window">Context Window (optional)</Label>
            <Input
              id="context-window"
              type="number"
              value={contextWindow}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setContextWindow(e.target.value)}
              placeholder="128000"
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              type="checkbox"
              id="model-is-default"
              checked={isDefault}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setIsDefault(e.target.checked)}
              className="h-4 w-4"
            />
            <Label htmlFor="model-is-default">Set as default for this provider</Label>
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

function ProviderCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9 rounded-lg" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-24" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full mb-4" />
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}

export default function ProvidersPage() {
  const { data: providers = [], isLoading: providersLoading, error: providersError } = useLlmProviders();
  const { data: models = [], isLoading: modelsLoading, error: modelsError } = useLlmModels();
  const deleteProvider = useDeleteLlmProvider();
  const deleteModel = useDeleteLlmModel();

  const [addProviderOpen, setAddProviderOpen] = useState(false);
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [apiKeyProvider, setApiKeyProvider] = useState<LlmProvider | null>(null);

  const handleDeleteProvider = async (id: string) => {
    if (confirm("Are you sure you want to delete this provider? All associated models will also be deleted.")) {
      await deleteProvider.mutateAsync(id);
    }
  };

  const handleDeleteModel = async (id: string) => {
    if (confirm("Are you sure you want to delete this model?")) {
      await deleteModel.mutateAsync(id);
    }
  };

  return (
    <div className="space-y-8">
      {/* LLM Providers Section */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">LLM Providers</h2>
            <p className="text-sm text-muted-foreground">
              Configure the LLM providers that your agents can use.
            </p>
          </div>
          <Button onClick={() => setAddProviderOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Add Provider
          </Button>
        </div>

        {providersError && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-4">
            Failed to load providers: {providersError.message}
          </div>
        )}

        {providersLoading ? (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {[...Array(3)].map((_, i) => (
              <ProviderCardSkeleton key={i} />
            ))}
          </div>
        ) : providers.length === 0 ? (
          <Card className="p-8 text-center">
            <Server className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No providers configured</h3>
            <p className="text-muted-foreground mb-4">
              Add an LLM provider to start using AI models with your agents.
            </p>
            <Button onClick={() => setAddProviderOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              Add Provider
            </Button>
          </Card>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {providers.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                onDelete={handleDeleteProvider}
                onSetApiKey={setApiKeyProvider}
              />
            ))}
          </div>
        )}
      </section>

      {/* LLM Models Section */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">Models</h2>
            <p className="text-sm text-muted-foreground">
              Manage the models available from your configured providers.
            </p>
          </div>
          <Button onClick={() => setAddModelOpen(true)} disabled={providers.length === 0}>
            <Plus className="h-4 w-4 mr-2" />
            Add Model
          </Button>
        </div>

        {modelsError && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-4">
            Failed to load models: {modelsError.message}
          </div>
        )}

        {modelsLoading ? (
          <div className="space-y-2">
            {[...Array(3)].map((_, i) => (
              <Skeleton key={i} className="h-16 w-full" />
            ))}
          </div>
        ) : models.length === 0 ? (
          <Card className="p-8 text-center">
            <Cpu className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No models configured</h3>
            <p className="text-muted-foreground mb-4">
              {providers.length === 0
                ? "Add a provider first, then add models to it."
                : "Add models to your providers to use them with agents."}
            </p>
            {providers.length > 0 && (
              <Button onClick={() => setAddModelOpen(true)}>
                <Plus className="h-4 w-4 mr-2" />
                Add Model
              </Button>
            )}
          </Card>
        ) : (
          <div className="space-y-2">
            {models.map((model) => (
              <ModelRow key={model.id} model={model} onDelete={handleDeleteModel} />
            ))}
          </div>
        )}
      </section>

      {/* Dialogs */}
      <AddProviderDialog open={addProviderOpen} onOpenChange={setAddProviderOpen} />
      <SetApiKeyDialog
        provider={apiKeyProvider}
        open={apiKeyProvider !== null}
        onOpenChange={(open) => !open && setApiKeyProvider(null)}
      />
      <AddModelDialog
        providers={providers}
        open={addModelOpen}
        onOpenChange={setAddModelOpen}
      />
    </div>
  );
}
