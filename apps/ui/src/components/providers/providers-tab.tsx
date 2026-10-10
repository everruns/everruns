"use client";

import { useMemo, useState } from "react";
import { Plus } from "lucide-react";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import { ProviderCard, ProviderCardSkeleton } from "./provider-card";
import { SetApiKeyDialog } from "./provider-dialogs";
import { useSyncProviderModels } from "@/hooks/use-providers";
import type { ModelWithProvider, Provider } from "@/lib/api/types";

export function ProvidersTab({
  providers,
  providersLoading,
  providersError,
  models,
  modelsLoading,
  canManage,
  onConnect,
  onReviewNew,
  onNotice,
}: {
  providers: Provider[];
  providersLoading: boolean;
  providersError: Error | null;
  models: ModelWithProvider[];
  modelsLoading: boolean;
  canManage: boolean;
  onConnect: () => void;
  onReviewNew: (provider: Provider) => void;
  onNotice: (notice: { kind: "success" | "error"; text: string }) => void;
}) {
  const syncModels = useSyncProviderModels();
  const [apiKeyProvider, setApiKeyProvider] = useState<Provider | null>(null);
  const [syncingId, setSyncingId] = useState<string | null>(null);

  const modelsByProvider = useMemo(() => {
    const grouped = new Map<string, ModelWithProvider[]>();
    for (const model of models) {
      const list = grouped.get(model.provider_id) ?? [];
      list.push(model);
      grouped.set(model.provider_id, list);
    }
    return grouped;
  }, [models]);

  const handleSync = async (provider: Provider) => {
    setSyncingId(provider.id);
    try {
      const result = await syncModels.mutateAsync(provider.id);
      if (result.status === "success") {
        onNotice({
          kind: "success",
          text:
            `${provider.name} synced: ${result.created} new, ${result.updated} updated, ` +
            `${result.stale} no longer listed.` +
            (result.created > 0 ? " Review the new ones on its card." : ""),
        });
      } else {
        onNotice({ kind: "error", text: `${provider.name} does not support model sync.` });
      }
    } catch {
      onNotice({ kind: "error", text: `Failed to sync ${provider.name}.` });
    } finally {
      setSyncingId(null);
    }
  };

  return (
    <>
      {providersError && (
        <Notice variant="destructive">
          <NoticeDescription>Failed to load providers: {providersError.message}</NoticeDescription>
        </Notice>
      )}
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          Only organization admins connect, rotate or delete providers. You can see what is
          connected and which models it serves.
        </p>
      )}
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {providersLoading
          ? [...Array(3)].map((_, index) => <ProviderCardSkeleton key={index} />)
          : providers.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                models={modelsByProvider.get(provider.id) ?? []}
                modelsLoading={modelsLoading}
                siblings={
                  providers.filter((p) => p.provider_type === provider.provider_type).length
                }
                canManage={canManage}
                isSyncing={syncingId === provider.id}
                onSetApiKey={setApiKeyProvider}
                onSyncModels={(p) => void handleSync(p)}
                onReviewNew={onReviewNew}
              />
            ))}
        {canManage && !providersLoading && (
          <button
            type="button"
            onClick={onConnect}
            className="flex min-h-48 flex-col items-center justify-center gap-2 border border-dashed text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <Plus className="icon-sharp size-5" />
            Connect provider
          </button>
        )}
      </div>
      {!providersLoading && providers.length === 0 && !canManage && (
        <p className="text-sm text-muted-foreground">
          No providers are connected yet. Ask an organization admin to connect one.
        </p>
      )}
      <SetApiKeyDialog
        provider={apiKeyProvider}
        open={apiKeyProvider !== null}
        onOpenChange={(open) => !open && setApiKeyProvider(null)}
      />
    </>
  );
}
