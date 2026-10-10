"use client";

// Registries → Models: models, the providers that serve them, and org defaults.
//
// Providers moved here from Settings: they are something people build and
// maintain, like agents or MCP servers, and connecting one, syncing it and
// choosing its models is one task. One page with three tabs keeps that task in
// one place (knowledge/ui/models-and-providers.md).
//
// The tab, provider filter and status filter live in the URL so links from
// elsewhere (`/models?tab=providers`, `/models?provider=…`) land on the view
// they name. `/settings/providers` redirects here.

import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useRouter, useSearchParams } from "next/navigation";
import { Cpu, Plus, PlugZap, SlidersHorizontal } from "lucide-react";
import { ModelsIcon } from "@/components/icons/facet-icons";
import { Button } from "@/components/ui/button";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import {
  PageBreadcrumb,
  PageContainer,
  PageMain,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { AddModelDialog } from "@/components/models/add-model-dialog";
import { DefaultsTab } from "@/components/models/defaults-tab";
import { ModelSelectionDrawer } from "@/components/models/model-selection";
import { ModelsTab, type StatusFilter } from "@/components/models/models-tab";
import { ConnectProviderSheet } from "@/components/providers/connect-provider-sheet";
import { getProviderLabel } from "@/components/providers/provider-icon";
import { ProvidersTab } from "@/components/providers/providers-tab";
import { usePageTitle } from "@/hooks";
import { useOrganization } from "@/hooks/use-organizations";
import { usePolicies } from "@/hooks/use-policies";
import {
  useDecisionDefault,
  useDeleteModel,
  useModels,
  useProviders,
  useReviewProviderModels,
  useSetModelsEnabled,
} from "@/hooks/use-providers";
import { ApiError } from "@/lib/api/client";
import { updateModel } from "@/lib/api/providers";
import type { DriverId, ModelWithProvider, Provider } from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { selectionChanges } from "@/lib/model-selection";

type Tab = "models" | "providers" | "defaults";
type PageNotice = { kind: "success" | "error"; text: string };

function errorDetail(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  if (error instanceof Error && error.message) return error.message;
  return "Unexpected error";
}

type Selection =
  | { kind: "review"; models: ModelWithProvider[] }
  | { kind: "choose"; provider: Provider };

export default function ModelsPage() {
  usePageTitle("Models");
  const queryClient = useQueryClient();
  const router = useRouter();
  const searchParams = useSearchParams();
  const tabParam = searchParams.get("tab");
  const tab: Tab = tabParam === "providers" || tabParam === "defaults" ? tabParam : "models";
  const providerId = searchParams.get("provider");
  const statusParam = searchParams.get("status");

  const {
    data: providers = [],
    isLoading: providersLoading,
    error: providersError,
  } = useProviders();
  const { data: models = [], isLoading: modelsLoading, error: modelsError } = useModels();
  const { data: org } = useOrganization();
  const decisionDefault = useDecisionDefault();
  const deleteModel = useDeleteModel();
  const setModelsEnabled = useSetModelsEnabled();
  const reviewModels = useReviewProviderModels();
  // Providers and models share one permission (`org:models:manage`).
  const { can } = usePolicies("models");
  const canManage = can("model.manage");

  const [connectOpen, setConnectOpen] = useState(false);
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [togglingModelId, setTogglingModelId] = useState<string | null>(null);
  const [notice, setNotice] = useState<PageNotice | null>(null);

  // Enabled models are the default view once there are any: with hundreds of
  // discovered models, the unfiltered list is mostly noise.
  const hasEnabled = models.some((model) => model.enabled);
  const status: StatusFilter =
    statusParam === "all" || statusParam === "enabled" || statusParam === "available"
      ? statusParam
      : hasEnabled
        ? "enabled"
        : "all";

  const setParams = (changes: Record<string, string | null>) => {
    const params = new URLSearchParams(searchParams.toString());
    for (const [key, value] of Object.entries(changes)) {
      if (value === null) params.delete(key);
      else params.set(key, value);
    }
    router.replace(`/models${params.size ? `?${params}` : ""}`, { scroll: false });
  };

  // Back from a provider's sign-in page (`?connected=<driver>`).
  const connected = searchParams.get("connected");
  useEffect(() => {
    if (!connected) return;
    setNotice({
      kind: "success",
      text: `${getProviderLabel(connected as DriverId)} connected. Review its models from the provider card.`,
    });
    const params = new URLSearchParams(searchParams.toString());
    params.delete("connected");
    router.replace(`/models${params.size ? `?${params}` : ""}`, { scroll: false });
  }, [connected, router, searchParams]);

  const counts = useMemo(
    () => ({ models: models.filter((model) => !model.stale).length, providers: providers.length }),
    [models, providers],
  );

  // A failed action is shown, never left as an unhandled rejection (EVE-954),
  // and the next action that succeeds clears it.
  const run = async (summary: string, action: () => Promise<void>) => {
    try {
      await action();
      setNotice((current) => (current?.kind === "error" ? null : current));
    } catch (error) {
      console.error(`${summary}:`, error);
      setNotice({ kind: "error", text: `${summary}: ${errorDetail(error)}` });
    }
  };

  const handleToggleEnabled = async (modelId: string, enabled: boolean) => {
    setTogglingModelId(modelId);
    await run(`Failed to ${enabled ? "enable" : "disable"} model`, async () => {
      await updateModel(modelId, { enabled });
      await queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      if (!enabled) await queryClient.invalidateQueries({ queryKey: queryKeys.organizations.all });
    });
    setTogglingModelId(null);
  };

  const handleUpdate = async (modelId: string, data: Parameters<typeof updateModel>[1]) => {
    let ok = false;
    await run("Failed to update model", async () => {
      await updateModel(modelId, data);
      await queryClient.invalidateQueries({ queryKey: queryKeys.models.all });
      ok = true;
    });
    return ok;
  };

  const handleDelete = async (modelId: string) => {
    if (!confirm("Are you sure you want to delete this model?")) return;
    await run("Failed to delete model", () => deleteModel.mutateAsync(modelId));
  };

  const selectionModels =
    selection?.kind === "review"
      ? selection.models
      : selection?.kind === "choose"
        ? models.filter((model) => model.provider_id === selection.provider.id)
        : [];

  const applySelection = async (selected: Set<string>) => {
    setSelectionError(null);
    const changes = selectionChanges(selectionModels, selected);
    const { failed } = await setModelsEnabled.mutateAsync(changes);
    if (failed.length > 0) {
      setSelectionError(
        `${failed.length} of ${changes.length} models could not be updated. Try again.`,
      );
      return;
    }
    // Choosing or reviewing a provider's models settles what is new about it.
    const reviewed = [...new Set(selectionModels.map((model) => model.provider_id))];
    await reviewModels.mutateAsync(reviewed).catch(() => undefined);
    const enabledNow = selectionModels.filter((model) => selected.has(model.id)).length;
    setNotice({
      kind: "success",
      text:
        selection?.kind === "choose"
          ? `${selection.provider.name}: ${enabledNow} of ${selectionModels.length} models enabled.`
          : `${enabledNow} of ${selectionModels.length} new models enabled.`,
    });
    setSelection(null);
  };

  const tabs = [
    {
      value: "models",
      label: "Models",
      icon: <Cpu className="icon-sharp size-3.5" />,
      count: counts.models,
    },
    {
      value: "providers",
      label: "Providers",
      icon: <PlugZap className="icon-sharp size-3.5" />,
      count: counts.providers,
    },
    {
      value: "defaults",
      label: "Defaults",
      icon: <SlidersHorizontal className="icon-sharp size-3.5" />,
    },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Registries" }, { label: "Models" }]} />
      <PageMasthead
        icon={<ModelsIcon />}
        title="Models"
        description="Connect providers and choose which of their models your agents can use."
        actions={
          canManage ? (
            <Button variant="accent" onClick={() => setConnectOpen(true)}>
              <Plus className="size-4" />
              Connect provider
            </Button>
          ) : undefined
        }
      />

      <SectionTabs
        aria-label="Models sections"
        value={tab}
        onValueChange={(value) => setParams({ tab: value === "models" ? null : value })}
        items={tabs}
      />

      {notice && (
        <Notice
          variant={notice.kind === "success" ? "success" : "destructive"}
          role={notice.kind === "success" ? "status" : "alert"}
        >
          <NoticeDescription className="flex items-center gap-3">
            <span className="flex-1">{notice.text}</span>
            <button
              type="button"
              className="text-xs text-muted-foreground hover:text-foreground"
              onClick={() => setNotice(null)}
            >
              Dismiss
            </button>
          </NoticeDescription>
        </Notice>
      )}

      <PageMain>
        {tab === "models" && (
          <ModelsTab
            models={models}
            providers={providers}
            modelsLoading={modelsLoading}
            modelsError={modelsError}
            providerId={providerId}
            onProviderChange={(id) => setParams({ provider: id })}
            status={status}
            onStatusChange={(value) => setParams({ status: value })}
            canManage={canManage}
            defaultModelId={org?.default_model_id ?? null}
            decisionDefault={decisionDefault.data ?? null}
            onGoToDefaults={() => setParams({ tab: "defaults" })}
            onConnect={() => setConnectOpen(true)}
            onAddModel={() => setAddModelOpen(true)}
            onReviewNew={(newModels) => {
              setSelectionError(null);
              setSelection({ kind: "review", models: newModels });
            }}
            onChooseModels={(provider) => {
              setSelectionError(null);
              setSelection({ kind: "choose", provider });
            }}
            onToggleEnabled={(id, enabled) => void handleToggleEnabled(id, enabled)}
            togglingModelId={togglingModelId}
            onUpdate={handleUpdate}
            onDelete={(id) => void handleDelete(id)}
          />
        )}
        {tab === "providers" && (
          <ProvidersTab
            providers={providers}
            providersLoading={providersLoading}
            providersError={providersError}
            models={models}
            modelsLoading={modelsLoading}
            canManage={canManage}
            onConnect={() => setConnectOpen(true)}
            onReviewNew={(provider) => {
              setSelectionError(null);
              setSelection({
                kind: "review",
                models: models.filter((model) => model.provider_id === provider.id && model.is_new),
              });
            }}
            onNotice={setNotice}
          />
        )}
        {tab === "defaults" && (
          <DefaultsTab models={models} providers={providers} canManage={canManage} />
        )}
      </PageMain>

      <ConnectProviderSheet
        open={connectOpen}
        onOpenChange={setConnectOpen}
        providers={providers}
        onConnected={(text) => {
          setNotice({ kind: "success", text });
          setParams({ tab: "providers" });
        }}
      />
      <ModelSelectionDrawer
        open={selection !== null}
        onOpenChange={(open) => !open && setSelection(null)}
        title={
          selection?.kind === "choose"
            ? `Choose ${selection.provider.name} models`
            : "Review new models"
        }
        description={
          selection?.kind === "choose"
            ? "Ticked models are enabled for agents. Unticking one disables it."
            : "These models appeared since the last review. Recommended ones are ticked."
        }
        models={selectionModels}
        mode={selection?.kind === "choose" ? "edit" : "enable"}
        recommend={selection?.kind === "review"}
        showProvider={selection?.kind === "review"}
        onApply={(selected) => void applySelection(selected)}
        applying={setModelsEnabled.isPending || reviewModels.isPending}
        error={selectionError}
      />
      <AddModelDialog providers={providers} open={addModelOpen} onOpenChange={setAddModelOpen} />
    </PageContainer>
  );
}
