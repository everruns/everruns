"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/components/ui/select";
import { useQueryClient } from "@tanstack/react-query";
import {
  useLlmProviders,
  useLlmModels,
  useDeleteLlmProvider,
  useSyncProviderModels,
  useDeleteLlmModel,
} from "@/hooks/use-llm-providers";
import { updateLlmModel } from "@/lib/api/llm-providers";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { queryKeys } from "@/lib/query-keys";
import { Plus, Server, Cpu } from "lucide-react";
import { ProviderIcon } from "@/components/providers/provider-icon";
import type { LlmProvider } from "@/lib/api/types";

import { ProviderCard, ProviderCardSkeleton } from "./provider-card";
import { ModelRow } from "./model-row";
import { AddProviderDialog, SetApiKeyDialog, AddModelDialog } from "./provider-dialogs";

export default function ProvidersPage() {
  const queryClient = useQueryClient();
  const {
    data: providers = [],
    isLoading: providersLoading,
    error: providersError,
  } = useLlmProviders();
  const { data: models = [], isLoading: modelsLoading, error: modelsError } = useLlmModels();
  const { data: org } = useOrganization();
  const updateOrg = useUpdateOrganization();
  const deleteProvider = useDeleteLlmProvider();
  const deleteModel = useDeleteLlmModel();
  const syncModels = useSyncProviderModels();
  const [togglingModelId, setTogglingModelId] = useState<string | null>(null);

  const [addProviderOpen, setAddProviderOpen] = useState(false);
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [apiKeyProvider, setApiKeyProvider] = useState<LlmProvider | null>(null);
  const [syncingProviderId, setSyncingProviderId] = useState<string | null>(null);
  const [syncMessage, setSyncMessage] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);

  const enabledModels = models.filter((m) => m.enabled);

  const handleDeleteProvider = async (id: string) => {
    if (
      confirm(
        "Are you sure you want to delete this provider? All associated models will also be deleted.",
      )
    ) {
      await deleteProvider.mutateAsync(id);
    }
  };

  const handleDeleteModel = async (id: string) => {
    if (confirm("Are you sure you want to delete this model?")) {
      await deleteModel.mutateAsync(id);
    }
  };

  const handleToggleEnabled = async (modelId: string, enabled: boolean) => {
    setTogglingModelId(modelId);
    try {
      await updateLlmModel(modelId, { enabled });
      await queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
      // If disabling, also refresh org in case it was the default
      if (!enabled) {
        await queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
      }
    } finally {
      setTogglingModelId(null);
    }
  };

  const handleSetDefaultModel = async (modelId: string) => {
    await updateOrg.mutateAsync({ default_model_id: modelId || undefined });
  };

  const handleSyncModels = async (providerId: string) => {
    setSyncingProviderId(providerId);
    setSyncMessage(null);
    try {
      const result = await syncModels.mutateAsync(providerId);
      if (result.status === "success") {
        setSyncMessage({
          type: "success",
          text: `Sync complete: ${result.created} new, ${result.updated} updated, ${result.stale} stale`,
        });
      } else {
        setSyncMessage({
          type: "error",
          text: "Model sync not supported for this provider",
        });
      }
    } catch {
      setSyncMessage({
        type: "error",
        text: "Failed to sync models",
      });
    } finally {
      setSyncingProviderId(null);
      // Clear message after 5 seconds
      setTimeout(() => setSyncMessage(null), 5000);
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
          <div className="bg-destructive/10 text-destructive p-4 mb-4">
            Failed to load providers: {providersError.message}
          </div>
        )}

        {syncMessage && (
          <div
            className={`p-4 mb-4 ${
              syncMessage.type === "success"
                ? "bg-green-100 text-green-800"
                : "bg-destructive/10 text-destructive"
            }`}
          >
            {syncMessage.text}
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
                onSyncModels={handleSyncModels}
                isSyncing={syncingProviderId === provider.id}
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
          <div className="bg-destructive/10 text-destructive p-4 mb-4">
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
              <ModelRow
                key={model.id}
                model={model}
                onDelete={handleDeleteModel}
                onToggleEnabled={handleToggleEnabled}
                isTogglingEnabled={togglingModelId === model.id}
              />
            ))}
          </div>
        )}
      </section>

      {/* Organization Default Model Section */}
      {enabledModels.length > 0 && (
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Organization Settings</h2>
            <p className="text-sm text-muted-foreground">
              Configure the default model for your organization. This is used when no model is
              specified at the agent or session level.
            </p>
          </div>
          <Card>
            <CardContent className="pt-6">
              <div className="flex items-center gap-4">
                <Label htmlFor="default-model" className="whitespace-nowrap font-medium">
                  Default Model
                </Label>
                <Select
                  value={org?.default_model_id ?? "none"}
                  onValueChange={(val) => handleSetDefaultModel(val === "none" ? "" : val)}
                  disabled={updateOrg.isPending}
                >
                  <SelectTrigger className="w-full max-w-md" id="default-model">
                    <span>
                      {org?.default_model_id
                        ? (enabledModels.find((m) => m.id === org.default_model_id)
                            ?.display_name ?? "Unknown model")
                        : "No default model"}
                    </span>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="none">No default model</SelectItem>
                    {enabledModels.map((model) => (
                      <SelectItem key={model.id} value={model.id}>
                        <div className="flex items-center gap-2">
                          <ProviderIcon
                            providerType={model.provider_type}
                            size="sm"
                            showBackground={false}
                          />
                          <span>
                            {model.display_name} ({model.provider_name})
                          </span>
                        </div>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </CardContent>
          </Card>
        </section>
      )}

      {/* Dialogs */}
      <AddProviderDialog open={addProviderOpen} onOpenChange={setAddProviderOpen} />
      <SetApiKeyDialog
        provider={apiKeyProvider}
        open={apiKeyProvider !== null}
        onOpenChange={(open) => !open && setApiKeyProvider(null)}
      />
      <AddModelDialog providers={providers} open={addModelOpen} onOpenChange={setAddModelOpen} />
    </div>
  );
}
