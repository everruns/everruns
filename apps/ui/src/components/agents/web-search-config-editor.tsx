"use client";

// Bespoke editor for the `web_search` capability config (mirrors
// crates/integrations/src/web_search/config.rs). A provider picker with the
// organization's real accounts, a plain-words "How this agent will search"
// box resolved by the server for the agent's default model (the same plan
// resolution the running tool uses), then results, freshness and domains.
// Unset fields follow the organization's defaults, so the editor writes only
// what the author changed.

import { useMemo } from "react";
import { Route } from "lucide-react";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { TagInput } from "@/components/ui/tag-input";
import { useOrganization } from "@/hooks/use-organizations";
import { useModels } from "@/hooks/use-providers";
import { useSearchPlanPreview, useSearchProviders } from "@/hooks/use-search-providers";
import type { SearchProviderSetting } from "@/lib/api/types";
import {
  SETTING_LABELS,
  WEB_SEARCH_MAX_RESULTS,
  accountLabel,
  describePlan,
  isValidDomain,
  parseDomainList,
} from "@/lib/web-search";
import { useCapabilityEditorContext } from "./capability-editor-context";

export interface WebSearchConfig {
  provider?: SearchProviderSetting;
  account_id?: string;
  fallback?: "next" | "none";
  limit?: number;
  freshness?: "day" | "week" | "month" | "year";
  allowed_domains?: string[];
  blocked_domains?: string[];
  show_provider?: boolean;
}

const FRESHNESS_LABELS: Record<string, string> = {
  any: "Any time",
  day: "Past day",
  week: "Past week",
  month: "Past month",
  year: "Past year",
};

/** Drop unset and default values so the stored config stays minimal. */
export function cleanWebSearchConfig(config: WebSearchConfig): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (config.provider && config.provider !== "org_default") out.provider = config.provider;
  if (config.provider === "account" && config.account_id) out.account_id = config.account_id;
  if (config.fallback) out.fallback = config.fallback;
  if (config.limit) out.limit = config.limit;
  if (config.freshness) out.freshness = config.freshness;
  if (config.allowed_domains?.length) out.allowed_domains = config.allowed_domains;
  if (config.blocked_domains?.length) out.blocked_domains = config.blocked_domains;
  if (config.show_provider) out.show_provider = true;
  return out;
}

function readConfig(config: unknown): WebSearchConfig {
  if (!config || typeof config !== "object" || Array.isArray(config)) return {};
  return config as WebSearchConfig;
}

function DomainField({
  id,
  label,
  value,
  onChange,
  disabled,
}: {
  id: string;
  label: string;
  value: string[];
  onChange: (value: string[]) => void;
  disabled?: boolean;
}) {
  const invalid = value.find((domain) => !isValidDomain(domain));
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} className="text-xs font-normal text-muted-foreground">
        {label}
      </Label>
      <TagInput
        id={id}
        value={value.join(", ")}
        onChange={(next) => onChange(parseDomainList(next))}
        placeholder="example.com"
        disabled={disabled}
      />
      {invalid && (
        <p className="text-xs text-destructive">
          &quot;{invalid}&quot; is not a bare host name like example.com.
        </p>
      )}
    </div>
  );
}

export function WebSearchConfigEditor({
  config,
  onChange,
  disabled,
}: {
  config: unknown;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}) {
  const current = readConfig(config);
  const { modelId } = useCapabilityEditorContext();
  const catalog = useSearchProviders();
  const { data: org } = useOrganization();
  const { data: models = [] } = useModels();
  const accounts = catalog.data?.accounts ?? [];
  const defaults = catalog.data?.defaults;

  const update = (patch: Partial<WebSearchConfig>) =>
    onChange(cleanWebSearchConfig({ ...current, ...patch }));

  const provider = current.provider ?? "org_default";
  const value =
    provider === "account" && current.account_id ? `account:${current.account_id}` : provider;

  const orgDefaultLabel = (() => {
    if (!defaults) return SETTING_LABELS.auto;
    if (defaults.provider === "account") {
      const account = accounts.find((candidate) => candidate.id === defaults.account_id);
      return account ? accountLabel(account) : SETTING_LABELS.account;
    }
    return SETTING_LABELS[defaults.provider ?? "auto"];
  })();

  const choices: { value: string; label: string; hint?: string; disabled?: boolean }[] = [
    { value: "org_default", label: `Organization default · ${orgDefaultLabel}` },
    { value: "auto", label: "Automatic", hint: "best available" },
    { value: "native", label: "Model provider search", hint: "needs a supported model" },
    ...accounts.map((account) => ({
      value: `account:${account.id}`,
      label: accountLabel(account),
      hint: account.status === "ready" ? "account" : "needs a key",
      disabled: account.status !== "ready" && value !== `account:${account.id}`,
    })),
    ...(provider === "brave" || provider === "parallel"
      ? [{ value: provider, label: SETTING_LABELS[provider], hint: "user or session key" }]
      : []),
    { value: "free", label: "Free search", hint: "no key" },
  ];
  if (!choices.some((choice) => choice.value === value)) {
    choices.push({ value, label: "Search account (not connected)", disabled: true });
  }

  // Resolve only valid configs: an account choice always carries its id here.
  const cleaned = cleanWebSearchConfig(current);
  const preview = useSearchPlanPreview({
    config: cleaned,
    model_id: modelId || undefined,
  });
  const modelName = useMemo(() => {
    const id = modelId || org?.default_model_id;
    return models.find((model) => model.id === id)?.display_name;
  }, [modelId, org?.default_model_id, models]);
  const planLines = preview.data ? describePlan(preview.data, modelName) : [];
  const orgLimit = defaults?.results_per_search ?? 8;

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1.5">
        <Label htmlFor="web-search-provider" className="text-xs font-normal text-muted-foreground">
          Search provider
        </Label>
        <Select
          value={value}
          onValueChange={(next) => {
            if (!next) return;
            if (next.startsWith("account:")) {
              update({ provider: "account", account_id: next.slice("account:".length) });
            } else {
              update({ provider: next as SearchProviderSetting, account_id: undefined });
            }
          }}
          disabled={disabled}
        >
          <SelectTrigger id="web-search-provider" className="w-full">
            <SelectValue>{choices.find((choice) => choice.value === value)?.label}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {choices.map((choice) => (
              <SelectItem key={choice.value} value={choice.value} disabled={choice.disabled}>
                <span className="flex w-full items-center justify-between gap-4">
                  <span>{choice.label}</span>
                  {choice.hint && (
                    <span className="text-xs text-muted-foreground">{choice.hint}</span>
                  )}
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div
        className="space-y-1 border bg-muted/40 px-3 py-2.5 text-xs leading-relaxed"
        aria-live="polite"
        data-testid="web-search-plan"
      >
        <div className="flex items-center gap-1.5 font-medium text-foreground">
          <Route className="size-3.5" aria-hidden="true" />
          How this agent will search
        </div>
        {preview.isLoading ? (
          <p className="text-muted-foreground">Working out the plan…</p>
        ) : preview.error ? (
          <p className="text-destructive">
            {preview.error instanceof Error ? preview.error.message : "Could not resolve the plan."}
          </p>
        ) : (
          planLines.map((line) => (
            <p key={line} className="text-muted-foreground">
              {line}
            </p>
          ))
        )}
      </div>

      <label className="flex items-start gap-2.5">
        <Switch
          aria-label="Tell the model which provider answered"
          checked={Boolean(current.show_provider)}
          onCheckedChange={(checked) => update({ show_provider: checked })}
          disabled={disabled}
        />
        <span className="text-xs">
          <span className="block">Tell the model which provider answered</span>
          <span className="block text-muted-foreground">
            Adds the provider and any fallback note to each result.
          </span>
        </span>
      </label>

      <div className="grid grid-cols-2 gap-2">
        <div className="space-y-1.5">
          <Label htmlFor="web-search-limit" className="text-xs font-normal text-muted-foreground">
            Results per search
          </Label>
          <Select
            value={current.limit ? String(current.limit) : "org"}
            onValueChange={(next) =>
              next && update({ limit: next === "org" ? undefined : Number(next) })
            }
            disabled={disabled}
          >
            <SelectTrigger id="web-search-limit" className="w-full">
              <SelectValue>
                {current.limit ? String(current.limit) : `Org default (${orgLimit})`}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="org">Org default ({orgLimit})</SelectItem>
              {Array.from({ length: WEB_SEARCH_MAX_RESULTS }, (_, i) => i + 1).map((count) => (
                <SelectItem key={count} value={String(count)}>
                  {count}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label
            htmlFor="web-search-freshness"
            className="text-xs font-normal text-muted-foreground"
          >
            Freshness
          </Label>
          <Select
            value={current.freshness ?? "any"}
            onValueChange={(next) =>
              next &&
              update({
                freshness: next === "any" ? undefined : (next as WebSearchConfig["freshness"]),
              })
            }
            disabled={disabled}
          >
            <SelectTrigger id="web-search-freshness" className="w-full">
              <SelectValue>{FRESHNESS_LABELS[current.freshness ?? "any"]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {Object.entries(FRESHNESS_LABELS).map(([key, label]) => (
                <SelectItem key={key} value={key}>
                  {label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="web-search-fallback" className="text-xs font-normal text-muted-foreground">
          When the provider cannot answer
        </Label>
        <Select
          value={current.fallback ?? "org"}
          onValueChange={(next) =>
            next &&
            update({ fallback: next === "org" ? undefined : (next as WebSearchConfig["fallback"]) })
          }
          disabled={disabled}
        >
          <SelectTrigger id="web-search-fallback" className="w-full">
            <SelectValue>
              {current.fallback === "none"
                ? "Stop: use only this provider"
                : current.fallback === "next"
                  ? "Fall back to the next provider, then free search"
                  : "Organization default"}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="org">Organization default</SelectItem>
            <SelectItem value="next">Fall back to the next provider, then free search</SelectItem>
            <SelectItem value="none">Stop: use only this provider</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <DomainField
        id="web-search-allowed"
        label="Only these sites"
        value={current.allowed_domains ?? []}
        onChange={(domains) => update({ allowed_domains: domains })}
        disabled={disabled}
      />
      <DomainField
        id="web-search-blocked"
        label="Never these sites"
        value={current.blocked_domains ?? []}
        onChange={(domains) => update({ blocked_domains: domains })}
        disabled={disabled}
      />
      {preview.data && preview.data.org_blocked_domains.length > 0 && (
        <p className="text-xs text-muted-foreground">
          The organization also blocks {preview.data.org_blocked_domains.join(", ")}.
        </p>
      )}
    </div>
  );
}
