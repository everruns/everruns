"use client";

import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  useModels,
  useProviders,
  useDeleteProvider,
  useSyncProviderModels,
} from "@/hooks/use-providers";
import { usePageTitle } from "@/hooks";
import { Plus, Server } from "lucide-react";
import type { Provider } from "@/lib/api/types";

import { ProviderCard, ProviderCardSkeleton } from "./provider-card";
import { AddProviderDialog, SetApiKeyDialog } from "./provider-dialogs";

export default function ProvidersPage() {
  usePageTitle("LLM Providers", "Settings");
  const {
    data: providers = [],
    isLoading: providersLoading,
    error: providersError,
  } = useProviders();
  const { data: models = [], isLoading: modelsLoading } = useModels();
  const deleteProvider = useDeleteProvider();
  const syncModels = useSyncProviderModels();

  const [addProviderOpen, setAddProviderOpen] = useState(false);
  const [apiKeyProvider, setApiKeyProvider] = useState<Provider | null>(null);
  const [syncingProviderId, setSyncingProviderId] = useState<string | null>(null);
  const [syncMessage, setSyncMessage] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);

  const modelCountsByProvider = useMemo(() => {
    const counts = new Map<string, { total: number; enabled: number }>();
    for (const model of models) {
      const current = counts.get(model.provider_id) ?? { total: 0, enabled: 0 };
      current.total += 1;
      if (model.enabled) current.enabled += 1;
      counts.set(model.provider_id, current);
    }
    return counts;
  }, [models]);

  const handleDeleteProvider = async (id: string) => {
    if (
      confirm(
        "Are you sure you want to delete this provider? All associated models will also be deleted.",
      )
    ) {
      await deleteProvider.mutateAsync(id);
    }
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
      setTimeout(() => setSyncMessage(null), 5000);
    }
  };

  return (
    <div className="space-y-8">
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
            {[...Array(3)].map((_, index) => (
              <ProviderCardSkeleton key={index} />
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
                modelCounts={modelCountsByProvider.get(provider.id) ?? { total: 0, enabled: 0 }}
                modelsLoading={modelsLoading}
              />
            ))}
          </div>
        )}
      </section>

      <AddProviderDialog open={addProviderOpen} onOpenChange={setAddProviderOpen} />
      <SetApiKeyDialog
        provider={apiKeyProvider}
        open={apiKeyProvider !== null}
        onOpenChange={(open) => !open && setApiKeyProvider(null)}
      />
    </div>
  );
}
