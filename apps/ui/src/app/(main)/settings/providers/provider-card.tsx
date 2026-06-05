"use client";

import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Key, Trash2, RefreshCw, Boxes, ExternalLink } from "lucide-react";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import { formatCountLabel } from "@/lib/formatting";
import type { LlmProvider } from "@/lib/api/types";

type ProviderModelCounts = {
  total: number;
  enabled: number;
};

export function ProviderCard({
  provider,
  onDelete,
  onSetApiKey,
  onSyncModels,
  isSyncing,
  modelCounts,
  modelsLoading,
}: {
  provider: LlmProvider;
  onDelete: (id: string) => void;
  onSetApiKey: (provider: LlmProvider) => void;
  onSyncModels: (id: string) => void;
  isSyncing: boolean;
  modelCounts: ProviderModelCounts;
  modelsLoading: boolean;
}) {
  const canSync =
    provider.api_key_set && (!provider.base_url || isOpenRouterUrl(provider.base_url));
  const modelsHref = `/models?provider=${encodeURIComponent(provider.id)}`;

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <ProviderIcon providerType={provider.provider_type} size="md" />
          <div>
            <CardTitle className="text-lg">
              <Link href={`/settings/providers/${provider.id}`} className="hover:underline">
                {provider.name}
              </Link>
            </CardTitle>
            <CardDescription className="text-sm">
              {getProviderLabel(provider.provider_type)}
            </CardDescription>
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
            <p className="text-muted-foreground truncate">URL: {provider.base_url}</p>
          )}
          <div className="flex items-center gap-2">
            <Key className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground">
              API Key: {provider.api_key_set ? "Configured" : "Not set"}
            </span>
          </div>
          <div className="flex items-start gap-2">
            <Boxes className="h-4 w-4 text-muted-foreground mt-0.5" />
            {modelsLoading ? (
              <Skeleton className="h-4 w-40" />
            ) : (
              <div className="text-muted-foreground">
                <span>
                  {formatCountLabel(modelCounts.total, "model")} available, {modelCounts.enabled}{" "}
                  enabled
                </span>
                <Link
                  href={modelsHref}
                  className="ml-2 inline-flex items-center gap-1 text-foreground hover:underline"
                >
                  View models
                  <ExternalLink className="h-3 w-3" />
                </Link>
              </div>
            )}
          </div>
        </div>
        <div className="flex items-center justify-end gap-2 mt-4">
          {canSync && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onSyncModels(provider.id)}
              disabled={isSyncing}
              title="Discover available models from provider API"
            >
              <RefreshCw className={`h-4 w-4 mr-1 ${isSyncing ? "animate-spin" : ""}`} />
              {isSyncing ? "Syncing..." : "Sync Models"}
            </Button>
          )}
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

function isOpenRouterUrl(baseUrl: string): boolean {
  try {
    const host = new URL(baseUrl).hostname.toLowerCase();
    return host === "openrouter.ai";
  } catch {
    return false;
  }
}

export function ProviderCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-24" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full mb-4" />
        <Skeleton className="h-4 w-2/3 mb-4" />
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}
