"use client";

import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardActions, CardContent, CardHeader } from "@/components/ui/card";
import { EntityCard, EntityCardDetail, EntityCardFooter } from "@/components/ui/entity-card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { IconTile } from "@/components/layout/page-layout";
import { Key, Trash2, RefreshCw, Boxes, Ellipsis, ExternalLink, Link2 } from "lucide-react";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import { getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import type { Provider } from "@/lib/api/types";

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
  provider: Provider;
  onDelete: (id: string) => void;
  onSetApiKey: (provider: Provider) => void;
  onSyncModels: (id: string) => void;
  isSyncing: boolean;
  modelCounts: ProviderModelCounts;
  modelsLoading: boolean;
}) {
  const canSync =
    provider.api_key_set && (!provider.base_url || isOpenRouterUrl(provider.base_url));
  const modelsHref = `/models?provider=${encodeURIComponent(provider.id)}`;
  const keyActionLabel = provider.api_key_set ? "Update key" : "Set key";
  // Host-managed providers are read-only to org admins (EVE-810): no credential
  // edits, no deletion. Model sync stays available.
  const canEdit = !provider.managed;

  const syncButton = canSync ? (
    <Button
      variant="outline"
      size="sm"
      onClick={() => onSyncModels(provider.id)}
      disabled={isSyncing}
      title="Discover available models from provider API"
    >
      <RefreshCw className={`icon-sharp h-4 w-4 mr-1 ${isSyncing ? "animate-spin" : ""}`} />
      {isSyncing ? "Syncing…" : "Sync models"}
    </Button>
  ) : null;

  const keyButton = canEdit ? (
    <Button variant="outline" size="sm" onClick={() => onSetApiKey(provider)}>
      <Key className="icon-sharp h-4 w-4 mr-1" />
      {keyActionLabel}
    </Button>
  ) : null;

  // Highest-priority action stays visible at every card width; the rest collapse
  // into an overflow menu when the card is too narrow for the full row.
  const primaryAction = syncButton ?? keyButton;
  const hasOverflow = canEdit && primaryAction !== keyButton;

  return (
    <EntityCard
      icon={
        <IconTile
          size="md"
          icon={
            <ProviderIcon
              providerType={provider.provider_type}
              size="sm"
              showBackground={false}
              className="p-0"
            />
          }
        />
      }
      title={provider.name}
      href={`/settings/providers/${provider.id}`}
      copyValue={provider.id}
      subtitle={
        <span className="text-xs text-muted-foreground">
          {getProviderLabel(provider.provider_type)}
        </span>
      }
      headerActions={
        <>
          {provider.managed && (
            <Badge variant="outline" title="Managed by the host">
              Managed
            </Badge>
          )}
          <Badge variant={getEntityStatusBadgeVariant(provider.status)}>{provider.status}</Badge>
        </>
      }
      footer={
        <EntityCardFooter
          className="mt-4"
          actions={
            (primaryAction || canEdit) && (
              <CardActions
                primary={primaryAction}
                expanded={
                  hasOverflow || canEdit ? (
                    <>
                      {hasOverflow && keyButton}
                      {canEdit && (
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          className="text-destructive"
                          aria-label="Delete provider"
                          onClick={() => onDelete(provider.id)}
                        >
                          <Trash2 className="icon-sharp h-4 w-4" />
                        </Button>
                      )}
                    </>
                  ) : null
                }
                collapsed={
                  canEdit ? (
                    <DropdownMenu>
                      <DropdownMenuTrigger
                        render={<Button variant="outline" size="icon-sm" />}
                        aria-label="More provider actions"
                      >
                        <Ellipsis className="icon-sharp" />
                      </DropdownMenuTrigger>
                      <DropdownMenuPositioner align="end">
                        <DropdownMenuContent>
                          {hasOverflow && (
                            <DropdownMenuItem onClick={() => onSetApiKey(provider)}>
                              <Key />
                              {keyActionLabel}
                            </DropdownMenuItem>
                          )}
                          <DropdownMenuItem
                            variant="destructive"
                            onClick={() => onDelete(provider.id)}
                          >
                            <Trash2 />
                            Delete
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenuPositioner>
                    </DropdownMenu>
                  ) : null
                }
              />
            )
          }
        />
      }
    >
      <div className="space-y-1.5">
        {provider.base_url && (
          <EntityCardDetail icon={<Link2 className="icon-sharp size-3.5" />} label="Endpoint">
            <span className="min-w-0 truncate font-mono" title={provider.base_url}>
              {provider.base_url}
            </span>
          </EntityCardDetail>
        )}
        <EntityCardDetail icon={<Key className="icon-sharp size-3.5" />} label="API key">
          <span className="truncate">{provider.api_key_set ? "Configured" : "Not set"}</span>
        </EntityCardDetail>
        <EntityCardDetail icon={<Boxes className="icon-sharp size-3.5" />} label="Models">
          {modelsLoading ? (
            <Skeleton className="h-4 w-40" />
          ) : (
            // Wraps instead of truncating so the count stays readable in a narrow card.
            <span className="flex min-w-0 flex-wrap items-center gap-x-1.5">
              <span>
                {modelCounts.total} available, {modelCounts.enabled} enabled
              </span>
              <Link
                href={modelsHref}
                className="inline-flex items-center gap-1 text-foreground hover:underline"
              >
                View models
                <ExternalLink className="icon-sharp h-3 w-3" />
              </Link>
            </span>
          )}
        </EntityCardDetail>
      </div>
    </EntityCard>
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

/** Mirrors {@link ProviderCard}'s EntityCard anatomy so loading does not shift layout. */
export function ProviderCardSkeleton() {
  return (
    <Card className="bg-background">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <Skeleton className="size-8 flex-shrink-0" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-3 w-24" />
          </div>
        </div>
        <Skeleton className="h-5 w-16 flex-shrink-0" />
      </CardHeader>
      <CardContent>
        <div className="space-y-1.5">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-4 w-56" />
        </div>
        <div className="mt-4 flex justify-end">
          <Skeleton className="h-8 w-28" />
        </div>
      </CardContent>
    </Card>
  );
}
