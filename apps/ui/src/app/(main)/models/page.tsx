"use client";

import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Cpu, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { PageShell, PageHeader, PageBody } from "@/components/layout";
import { AddModelDialog } from "@/components/models/add-model-dialog";
import { ModelRow } from "@/components/models/model-row";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { useModels, useProviders, useDeleteModel } from "@/hooks/use-providers";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { usePageTitle } from "@/hooks";
import { updateModel } from "@/lib/api/providers";
import { queryKeys } from "@/lib/query-keys";
import type { ModelWithProvider } from "@/lib/api/types";

// Order models by release date desc (newest first), then by created_at desc.
// Models without a release_date in their profile fall to the bottom; this works
// uniformly across providers because release_date comes from the shared
// models.dev profile.
function compareByRecency(a: ModelWithProvider, b: ModelWithProvider): number {
  const aDate = a.profile?.release_date ?? "";
  const bDate = b.profile?.release_date ?? "";
  if (aDate !== bDate) return bDate.localeCompare(aDate);
  return (b.created_at ?? "").localeCompare(a.created_at ?? "");
}

export default function ModelsPage() {
  usePageTitle("Models");
  const queryClient = useQueryClient();
  const searchParams = useSearchParams();
  const { data: providers = [] } = useProviders();
  const { data: models = [], isLoading: modelsLoading, error: modelsError } = useModels();
  const { data: org } = useOrganization();
  const updateOrg = useUpdateOrganization();
  const deleteModel = useDeleteModel();
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [togglingModelId, setTogglingModelId] = useState<string | null>(null);
  const selectedProviderId = searchParams.get("provider");
  const selectedProvider = useMemo(
    () =>
      selectedProviderId
        ? providers.find((provider) => provider.id === selectedProviderId)
        : undefined,
    [providers, selectedProviderId],
  );
  const filteredModels = useMemo(
    () =>
      selectedProviderId
        ? models.filter((model) => model.provider_id === selectedProviderId)
        : models,
    [models, selectedProviderId],
  );

  const { enabledModels, availableModels } = useMemo(() => {
    const enabled = filteredModels.filter((m) => m.enabled).sort(compareByRecency);
    const available = filteredModels.filter((m) => !m.enabled).sort(compareByRecency);
    return { enabledModels: enabled, availableModels: available };
  }, [filteredModels]);
  const allEnabledModels = useMemo(
    () => models.filter((model) => model.enabled).sort(compareByRecency),
    [models],
  );
  const selectedDefaultModelName = org?.default_model_id
    ? (allEnabledModels.find((model) => model.id === org.default_model_id)?.display_name ??
      "Unknown model")
    : undefined;

  const handleDeleteModel = async (id: string) => {
    if (confirm("Are you sure you want to delete this model?")) {
      await deleteModel.mutateAsync(id);
    }
  };

  const handleToggleEnabled = async (modelId: string, enabled: boolean) => {
    setTogglingModelId(modelId);
    try {
      await updateModel(modelId, { enabled });
      await queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      if (!enabled) {
        await queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
      }
    } finally {
      setTogglingModelId(null);
    }
  };

  const handleUpdateModel = async (modelId: string, data: Parameters<typeof updateModel>[1]) => {
    await updateModel(modelId, data);
    await queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
    await queryClient.invalidateQueries({ queryKey: queryKeys.providers.all });
  };

  const handleSetDefaultModel = async (modelId: string) => {
    await updateOrg.mutateAsync({ default_model_id: modelId || undefined });
  };

  return (
    <PageShell>
      <PageHeader
        title="Models"
        description={
          selectedProvider
            ? `Manage the models available from ${selectedProvider.name}.`
            : "Manage the models available from your configured providers."
        }
        actions={
          <>
            {selectedProviderId && (
              <Link href="/models">
                <Button variant="outline">Clear filter</Button>
              </Link>
            )}
            <Button onClick={() => setAddModelOpen(true)} disabled={providers.length === 0}>
              <Plus className="h-4 w-4 mr-2" />
              Add Model
            </Button>
          </>
        }
      />

      <PageBody>
        <section>
          {modelsError && (
            <div className="bg-destructive/10 text-destructive p-4 mb-4">
              Failed to load models: {modelsError.message}
            </div>
          )}

          {modelsLoading ? (
            <div className="space-y-2">
              {[...Array(3)].map((_, index) => (
                <Skeleton key={index} className="h-16 w-full" />
              ))}
            </div>
          ) : filteredModels.length === 0 ? (
            <Card className="p-8 text-center">
              <Cpu className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">
                {selectedProvider ? "No models for this provider" : "No models configured"}
              </h3>
              <p className="text-muted-foreground mb-4">
                {selectedProvider
                  ? "Sync or add models for this provider to use them with agents."
                  : providers.length === 0
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
            <div className="space-y-8">
              {enabledModels.length > 0 && (
                <div>
                  <div className="flex items-center justify-between mb-3">
                    <h2 className="text-xl font-semibold">Enabled models</h2>
                    <span className="text-sm text-muted-foreground">
                      {enabledModels.length} enabled
                    </span>
                  </div>
                  <div className="space-y-2">
                    {enabledModels.map((model) => (
                      <ModelRow
                        key={model.id}
                        model={model}
                        providers={providers}
                        onDelete={handleDeleteModel}
                        onUpdate={handleUpdateModel}
                        onToggleEnabled={handleToggleEnabled}
                        isTogglingEnabled={togglingModelId === model.id}
                      />
                    ))}
                  </div>
                </div>
              )}

              {enabledModels.length > 0 && availableModels.length > 0 && (
                <hr className="border-border" />
              )}

              {availableModels.length > 0 && (
                <div>
                  <div className="flex items-center justify-between mb-3">
                    <h2 className="text-xl font-semibold">Available models</h2>
                    <span className="text-sm text-muted-foreground">
                      Enable a model to use it with agents
                    </span>
                  </div>
                  <div className="space-y-2">
                    {availableModels.map((model) => (
                      <ModelRow
                        key={model.id}
                        model={model}
                        providers={providers}
                        onDelete={handleDeleteModel}
                        onUpdate={handleUpdateModel}
                        onToggleEnabled={handleToggleEnabled}
                        isTogglingEnabled={togglingModelId === model.id}
                      />
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </section>

        {allEnabledModels.length > 0 && (
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
                    onValueChange={(value) => handleSetDefaultModel(value === "none" ? "" : value)}
                    disabled={updateOrg.isPending}
                  >
                    <SelectTrigger className="w-full max-w-md" id="default-model">
                      <SelectValue placeholder="No default model">
                        {selectedDefaultModelName}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">No default model</SelectItem>
                      {allEnabledModels.map((model) => (
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
      </PageBody>

      <AddModelDialog providers={providers} open={addModelOpen} onOpenChange={setAddModelOpen} />
    </PageShell>
  );
}
