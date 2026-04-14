"use client";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Key, Trash2, RefreshCw } from "lucide-react";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import type { LlmProvider } from "@/lib/api/types";

export function ProviderCard({
  provider,
  onDelete,
  onSetApiKey,
  onSyncModels,
  isSyncing,
}: {
  provider: LlmProvider;
  onDelete: (id: string) => void;
  onSetApiKey: (provider: LlmProvider) => void;
  onSyncModels: (id: string) => void;
  isSyncing: boolean;
}) {
  // Only show sync button for providers without custom base URL (standard providers)
  const canSync = !provider.base_url && provider.api_key_set;

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <ProviderIcon providerType={provider.provider_type} size="md" />
          <div>
            <CardTitle className="text-lg">{provider.name}</CardTitle>
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
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}
