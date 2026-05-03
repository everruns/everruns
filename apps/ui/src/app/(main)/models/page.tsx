"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
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
import { useLlmModels, useLlmProviders, useDeleteLlmModel } from "@/hooks/use-llm-providers";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { updateLlmModel } from "@/lib/api/llm-providers";
import { queryKeys } from "@/lib/query-keys";

export default function ModelsPage() {
  const queryClient = useQueryClient();
  const { data: providers = [] } = useLlmProviders();
  const { data: models = [], isLoading: modelsLoading, error: modelsError } = useLlmModels();
  const { data: org } = useOrganization();
  const updateOrg = useUpdateOrganization();
  const deleteModel = useDeleteLlmModel();
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [togglingModelId, setTogglingModelId] = useState<string | null>(null);

  const enabledModels = models.filter((model) => model.enabled);
  const selectedDefaultModelName = org?.default_model_id
    ? (enabledModels.find((model) => model.id === org.default_model_id)?.display_name ??
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
      await updateLlmModel(modelId, { enabled });
      await queryClient.invalidateQueries({ queryKey: queryKeys.llmModels.all });
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

  return (
    <PageShell>
      <PageHeader
        title="Models"
        description="Manage the models available from your configured providers."
        actions={
          <Button onClick={() => setAddModelOpen(true)} disabled={providers.length === 0}>
            <Plus className="h-4 w-4 mr-2" />
            Add Model
          </Button>
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
      </PageBody>

      <AddModelDialog providers={providers} open={addModelOpen} onOpenChange={setAddModelOpen} />
    </PageShell>
  );
}
