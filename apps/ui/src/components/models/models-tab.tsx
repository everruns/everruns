"use client";

// Every model from every provider, filtered by service, provider and status.
//
// Built for catalogs of hundreds: enabled models are the default view, rows
// render a page at a time, and bulk changes go through the selection drawer
// ("Choose models" for a provider, "Review" for what a sync discovered) rather
// than row by row.

import { useMemo, useState } from "react";
import Link from "next/link";
import { Layers, MessageSquare, Plus, Scale, Binary, Sparkles, ListChecks } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { SearchInput } from "@/components/ui/search-input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { ModelsIcon } from "@/components/icons/facet-icons";
import { ModelRow } from "@/components/models/model-row";
import { cn } from "@/lib/utils";
import type {
  ModelService,
  ModelWithProvider,
  Provider,
  UpdateModelRequest,
} from "@/lib/api/types";
import { modelService } from "@/lib/model-capabilities";
import { SERVICE_LABELS, SERVICE_ORDER, compareByRecency, modelLabel } from "@/lib/model-selection";

export type StatusFilter = "all" | "enabled" | "available";

const PAGE_SIZE = 50;

const SERVICE_ICONS: Partial<Record<ModelService | "all", typeof Layers>> = {
  all: Layers,
  chat: MessageSquare,
  decisions: Scale,
  embeddings: Binary,
};

// Chat, decisions and embeddings are always offered; rarer services appear
// once a provider lists one.
const ALWAYS_SHOWN: ModelService[] = ["chat", "decisions", "embeddings"];

export function ModelsTab({
  models,
  providers,
  modelsLoading,
  modelsError,
  providerId,
  onProviderChange,
  status,
  onStatusChange,
  canManage,
  defaultModelId,
  decisionDefault,
  onGoToDefaults,
  onConnect,
  onAddModel,
  onReviewNew,
  onChooseModels,
  onToggleEnabled,
  togglingModelId,
  onUpdate,
  onDelete,
}: {
  models: ModelWithProvider[];
  providers: Provider[];
  modelsLoading: boolean;
  modelsError: Error | null;
  providerId: string | null;
  onProviderChange: (providerId: string | null) => void;
  status: StatusFilter;
  onStatusChange: (status: StatusFilter) => void;
  canManage: boolean;
  defaultModelId: string | null;
  decisionDefault: ModelWithProvider | null;
  onGoToDefaults: () => void;
  onConnect: () => void;
  onAddModel: () => void;
  onReviewNew: (models: ModelWithProvider[]) => void;
  onChooseModels: (provider: Provider) => void;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  togglingModelId: string | null;
  onUpdate: (id: string, data: UpdateModelRequest) => Promise<boolean>;
  onDelete: (id: string) => void;
}) {
  const [service, setService] = useState<ModelService | "all">("all");
  const [search, setSearch] = useState("");
  const [visible, setVisible] = useState(PAGE_SIZE);
  const selectedProvider = providers.find((provider) => provider.id === providerId);

  const providerModels = useMemo(
    () => (providerId ? models.filter((model) => model.provider_id === providerId) : models),
    [models, providerId],
  );
  const newModels = useMemo(() => providerModels.filter((model) => model.is_new), [providerModels]);

  // Service, provider and search narrow the scope; status splits it.
  const scope = useMemo(() => {
    const query = search.trim().toLowerCase();
    return providerModels.filter(
      (model) =>
        (service === "all" || modelService(model) === service) &&
        (!query ||
          `${model.display_name} ${model.model_id} ${model.provider_name}`
            .toLowerCase()
            .includes(query)),
    );
  }, [providerModels, service, search]);
  const enabledCount = scope.filter((model) => model.enabled).length;
  const shown = useMemo(
    () =>
      scope
        .filter((model) => status === "all" || (status === "enabled") === model.enabled)
        .sort((a, b) => Number(b.enabled) - Number(a.enabled) || compareByRecency(a, b)),
    [scope, status],
  );

  const serviceChips = (["all", ...SERVICE_ORDER] as const)
    .map((key) => ({
      key,
      label: key === "all" ? "All" : SERVICE_LABELS[key],
      count:
        key === "all"
          ? providerModels.length
          : providerModels.filter((model) => modelService(model) === key).length,
    }))
    .filter(
      (chip) =>
        chip.key === "all" ||
        ALWAYS_SHOWN.includes(chip.key) ||
        chip.count > 0 ||
        chip.key === service,
    );

  const resetPaging = () => setVisible(PAGE_SIZE);

  if (modelsError) {
    return (
      <div className="border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
        Failed to load models: {modelsError.message}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap gap-1.5" role="group" aria-label="Model service">
        {serviceChips.map((chip) => {
          const Icon = SERVICE_ICONS[chip.key] ?? Layers;
          const pressed = service === chip.key;
          return (
            <button
              key={chip.key}
              type="button"
              aria-pressed={pressed}
              onClick={() => {
                setService(chip.key);
                resetPaging();
              }}
              className={cn(
                "inline-flex h-8 items-center gap-1.5 border px-3 text-[13px] transition-colors hover:bg-muted",
                pressed ? "border-primary bg-primary/5 font-semibold" : "bg-card font-medium",
              )}
            >
              <Icon className="icon-sharp size-3.5" />
              {chip.label}
              <span className="font-mono text-xs font-normal text-muted-foreground">
                {chip.count}
              </span>
            </button>
          );
        })}
      </div>

      {service === "decisions" && (
        <div className="border bg-card px-4 py-3">
          <p className="text-[13px] font-medium">
            Decision models return structured verdicts for the Jev capability
          </p>
          <p className="text-xs text-muted-foreground">
            Only calibrated models can be the decision default. Default:{" "}
            <span className="font-medium text-foreground">
              {decisionDefault ? modelLabel(decisionDefault) : "None"}
            </span>{" "}
            ·{" "}
            <button type="button" className="text-primary hover:underline" onClick={onGoToDefaults}>
              Change in Defaults
            </button>
          </p>
        </div>
      )}

      {canManage && newModels.length > 0 && (
        <div className="flex flex-wrap items-center gap-3 border border-accent/30 bg-accent/10 px-4 py-2.5 text-sm">
          <Sparkles className="icon-sharp size-4 text-accent-foreground" />
          <span className="flex-1">
            {newModels.length} new {newModels.length === 1 ? "model" : "models"} discovered
            {selectedProvider ? ` by ${selectedProvider.name}` : ""}.
          </span>
          <Button size="sm" onClick={() => onReviewNew(newModels)}>
            Review
          </Button>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search models…"
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            resetPaging();
          }}
          containerClassName="w-64"
        />
        <Select
          value={providerId ?? "all"}
          onValueChange={(value) => {
            onProviderChange(value === "all" ? null : value);
            resetPaging();
          }}
        >
          <SelectTrigger aria-label="Provider" className="w-48">
            <SelectValue>{selectedProvider?.name ?? "All providers"}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All providers</SelectItem>
            {providers.map((provider) => (
              <SelectItem key={provider.id} value={provider.id}>
                {provider.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="inline-flex border" role="group" aria-label="Model status">
          {(
            [
              ["all", "All", scope.length],
              ["enabled", "Enabled", enabledCount],
              ["available", "Available", scope.length - enabledCount],
            ] as const
          ).map(([key, label, count]) => (
            <button
              key={key}
              type="button"
              aria-pressed={status === key}
              onClick={() => {
                onStatusChange(key);
                resetPaging();
              }}
              className={cn(
                "inline-flex h-8 items-center gap-1.5 px-3 text-[13px] transition-colors",
                status === key ? "bg-muted font-semibold" : "hover:bg-muted/60",
              )}
            >
              {label}
              <span className="font-mono text-xs font-normal text-muted-foreground">{count}</span>
            </button>
          ))}
        </div>
        {canManage && (
          <div className="ml-auto flex items-center gap-2">
            {selectedProvider && (
              <Button variant="outline" onClick={() => onChooseModels(selectedProvider)}>
                <ListChecks className="icon-sharp size-4" />
                Choose models
              </Button>
            )}
            <Button variant="outline" onClick={onAddModel} disabled={providers.length === 0}>
              <Plus className="icon-sharp size-4" />
              Add model
            </Button>
          </div>
        )}
      </div>

      {modelsLoading ? (
        <div className="space-y-2">
          {[...Array(4)].map((_, index) => (
            <Skeleton key={index} className="h-16 w-full" />
          ))}
        </div>
      ) : shown.length === 0 ? (
        <EmptyModels
          hasProviders={providers.length > 0}
          searching={search.trim() !== ""}
          status={status}
          availableCount={scope.length - enabledCount}
          canManage={canManage}
          onShowAll={() => onStatusChange("all")}
          onConnect={onConnect}
        />
      ) : (
        <div className="flex flex-col gap-2">
          {shown.slice(0, visible).map((model) => (
            <ModelRow
              key={model.id}
              model={model}
              providers={providers}
              canManage={canManage}
              onDelete={onDelete}
              onUpdate={onUpdate}
              onToggleEnabled={onToggleEnabled}
              isTogglingEnabled={togglingModelId === model.id}
              badges={
                <>
                  {model.id === defaultModelId && <Badge variant="secondary">Org default</Badge>}
                  {model.id === decisionDefault?.id && (
                    <Badge variant="secondary">Decision default</Badge>
                  )}
                </>
              }
            />
          ))}
          <div className="flex flex-wrap items-center gap-3 pt-1 text-sm text-muted-foreground">
            <span>
              Showing {Math.min(visible, shown.length)} of {shown.length}
            </span>
            {shown.length > visible && (
              <Button variant="outline" size="sm" onClick={() => setVisible((v) => v + PAGE_SIZE)}>
                Show {Math.min(PAGE_SIZE, shown.length - visible)} more
              </Button>
            )}
            {providerId && (
              <Link href="/models" className="ml-auto text-primary hover:underline">
                Clear provider filter →
              </Link>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function EmptyModels({
  hasProviders,
  searching,
  status,
  availableCount,
  canManage,
  onShowAll,
  onConnect,
}: {
  hasProviders: boolean;
  searching: boolean;
  status: StatusFilter;
  availableCount: number;
  canManage: boolean;
  onShowAll: () => void;
  onConnect: () => void;
}) {
  const [title, body] = !hasProviders
    ? [
        "No providers connected",
        canManage
          ? "Connect a provider to discover its models."
          : "Ask an organization admin to connect a provider.",
      ]
    : searching
      ? ["No models match your search", "Try a different search term or filter."]
      : status === "enabled" && availableCount > 0
        ? [
            "No enabled models here",
            `${availableCount} ${availableCount === 1 ? "model is" : "models are"} available to enable.`,
          ]
        : ["No models", "Sync a provider, or add a model by hand."];
  return (
    <Card className="p-8 text-center">
      <ModelsIcon className="mx-auto mb-4 h-12 w-12 text-muted-foreground" />
      <h3 className="mb-2 text-lg font-medium">{title}</h3>
      <p className="mb-4 text-muted-foreground">{body}</p>
      {!hasProviders && canManage && (
        <Button onClick={onConnect}>
          <Plus className="icon-sharp size-4" />
          Connect provider
        </Button>
      )}
      {hasProviders && !searching && status === "enabled" && availableCount > 0 && (
        <Button variant="outline" onClick={onShowAll}>
          Show all models
        </Button>
      )}
    </Card>
  );
}
