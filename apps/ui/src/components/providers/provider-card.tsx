"use client";

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";

import Link from "next/link";
import { Key, RefreshCw, Settings2, Sparkles } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { EntityCard, EntityCardDetail } from "@/components/ui/entity-card";
import { Skeleton } from "@/components/ui/skeleton";
import { IconTile } from "@/components/layout/page-layout";
import {
  ProviderIcon,
  getProviderDescription,
  getProviderLabel,
} from "@/components/providers/provider-icon";
import { formatRelativeTime } from "@/lib/formatting";
import { managedProviderCopy } from "@/lib/managed-provider-copy";
import { SERVICE_LABELS, SERVICE_ORDER } from "@/lib/model-selection";
import { modelService } from "@/lib/model-capabilities";
import type { ModelWithProvider, Provider } from "@/lib/api/types";

export type ProviderCardProps = {
  provider: Provider;
  /** This provider's models, stale ones included. */
  models: ModelWithProvider[];
  modelsLoading: boolean;
  /** How many providers share this one's driver (this one included). */
  siblings: number;
  /** Caller may connect, rotate keys, sync and choose models. */
  canManage: boolean;
  isSyncing: boolean;
  onSetApiKey: (provider: Provider) => void;
  onSyncModels: (provider: Provider) => void;
  onReviewNew: (provider: Provider) => void;
};

export function ProviderCard(props: ProviderCardProps) {
  return <ProviderCardContent {...props} />;
}

function providerStatus(provider: Provider): { label: string; dot: string } {
  if (provider.status === "disabled") return { label: "Disabled", dot: "bg-muted-foreground/50" };
  if (!provider.api_key_set) {
    return { label: "Needs a key", dot: "bg-warning" };
  }
  return { label: "Active", dot: "bg-success" };
}

function ProviderCardContent({
  provider,
  models,
  modelsLoading,
  siblings,
  canManage,
  isSyncing,
  onSetApiKey,
  onSyncModels,
  onReviewNew,
}: ProviderCardProps) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  const status = providerStatus(provider);
  const listed = models.filter((model) => !model.stale);
  const enabled = listed.filter((model) => model.enabled).length;
  const newCount = listed.filter((model) => model.is_new).length;
  const services = SERVICE_ORDER.filter((service) =>
    listed.some((model) => modelService(model) === service),
  ).map((service) => SERVICE_LABELS[service]);
  // Host-managed providers keep their credential and catalog with the host
  // (EVE-810); choosing which models to enable stays with the org.
  const canEditCredential = canManage && !provider.managed;
  const canSync = canManage && provider.api_key_set;
  const detailHref = `/models/providers/${provider.id}`;
  const modelsHref = `/models?provider=${encodeURIComponent(provider.id)}`;

  const credential = provider.managed
    ? "Host managed"
    : provider.api_key_set
      ? "Configured"
      : "Not set";

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
      href={detailHref}
      subtitle={
        <span className="text-xs text-muted-foreground">
          {getProviderLabel(provider.provider_type)}
          {siblings > 1 && (
            <>
              {" "}
              · {siblings} {ui("connected")}
            </>
          )}
        </span>
      }
      headerActions={
        <>
          {provider.managed && (
            <Badge variant="outline" title={managedProviderCopy.badgeTitle}>
              {managedProviderCopy.badge}
            </Badge>
          )}
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-muted-foreground">
            <span aria-hidden className={`size-1.5 rounded-full ${status.dot}`} />
            {status.label}
          </span>
        </>
      }
      footer={
        <div className="mt-4 flex items-center gap-1.5 border-t pt-3">
          {modelsLoading ? (
            <Skeleton className="mr-auto h-4 w-36" />
          ) : (
            <Link href={modelsHref} className="mr-auto text-xs text-primary hover:underline">
              {listed.length > 0
                ? ui("{value0} of {value1} models enabled →", {
                    value0: enabled,
                    value1: listed.length,
                  })
                : ui("No models yet")}
            </Link>
          )}
          {canEditCredential && !provider.api_key_set && (
            <Button size="sm" onClick={() => onSetApiKey(provider)}>
              <Key className="icon-sharp size-3.5" />
              {ui("Set key")}
            </Button>
          )}
          {canEditCredential && provider.api_key_set && (
            <Button
              variant="ghost"
              size="icon-sm"
              title={ui("Update key")}
              aria-label={ui("Update key for {value0}", { value0: provider.name })}
              onClick={() => onSetApiKey(provider)}
            >
              <Key className="icon-sharp size-3.5" />
            </Button>
          )}
          {canSync && (
            <Button
              variant="ghost"
              size="icon-sm"
              title={ui("Sync models")}
              aria-label={ui("Sync models for {value0}", { value0: provider.name })}
              onClick={() => onSyncModels(provider)}
              disabled={isSyncing}
            >
              <RefreshCw className={`icon-sharp size-3.5 ${isSyncing ? "animate-spin" : ""}`} />
            </Button>
          )}
          <LinkButton
            variant="ghost"
            size="icon-sm"
            href={detailHref}
            title={ui("Provider settings")}
            aria-label={ui("Settings for {value0}", { value0: provider.name })}
          >
            <Settings2 className="icon-sharp size-3.5" />
          </LinkButton>
        </div>
      }
    >
      <div className="space-y-3">
        <p className="text-sm text-foreground/75">
          {getProviderDescription(provider.provider_type)}
        </p>
        <div className="space-y-1.5">
          <EntityCardDetail label={<span className="inline-block w-20">{ui("Services")}</span>}>
            <span className="truncate">{services.join(", ") || "—"}</span>
          </EntityCardDetail>
          <EntityCardDetail label={<span className="inline-block w-20">{ui("Credentials")}</span>}>
            <span className={provider.api_key_set ? "truncate" : "truncate text-warning"}>
              {credential}
            </span>
          </EntityCardDetail>
          <EntityCardDetail label={<span className="inline-block w-20">{ui("Endpoint")}</span>}>
            <span className="min-w-0 truncate font-mono" title={provider.base_url}>
              {provider.base_url ?? ui("Driver default")}
            </span>
          </EntityCardDetail>
          <EntityCardDetail label={<span className="inline-block w-20">{ui("Last synced")}</span>}>
            <span>
              {provider.last_synced_at
                ? formatRelativeTime(provider.last_synced_at, displayLocale)
                : ui("Never")}
            </span>
          </EntityCardDetail>
        </div>
        {canManage && newCount > 0 && (
          <button
            type="button"
            onClick={() => onReviewNew(provider)}
            className="flex w-full items-center gap-2 border border-accent/30 bg-accent/10 px-3 py-2 text-left text-xs font-medium transition-colors hover:bg-accent/20"
          >
            <Sparkles className="icon-sharp size-3.5 text-accent-foreground" />
            {newCount} {ui("new")} {newCount === 1 ? ui("model") : ui("models")}
            <span className="ml-auto text-primary">{ui("Review")}</span>
          </button>
        )}
      </div>
    </EntityCard>
  );
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
          <Skeleton className="h-4 w-56" />
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-4 w-48" />
        </div>
        <div className="mt-4 flex justify-end">
          <Skeleton className="h-8 w-28" />
        </div>
      </CardContent>
    </Card>
  );
}
