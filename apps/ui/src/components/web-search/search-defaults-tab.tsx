"use client";

// Web search → Defaults: edits the organization's `web_search_defaults`
// (PATCH /v1/orgs/{org}). Every change saves the whole object at once, the
// way the server stores it, from a local draft so quick edits build on each
// other. The Automatic list reorders with up/down buttons (keyboard and
// screen-reader friendly); free search is pinned last and cannot move. The
// page remounts this tab when the stored defaults change, which resets the draft.

import { useState, type ReactNode } from "react";
import { ArrowDown, ArrowUp, Lock } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { TagInput } from "@/components/ui/tag-input";
import { useSaveWebSearchDefaults } from "@/hooks/use-search-providers";
import { ApiError } from "@/lib/api/client";
import type { SearchProviderCatalog, WebSearchDefaults } from "@/lib/api/types";
import {
  NATIVE_PRICE_SHORT,
  AUTOMATIC_HOSTED,
  WEB_SEARCH_DEFAULT_RESULTS,
  WEB_SEARCH_MAX_RESULTS,
  accountLabel,
  automaticEntries,
  automaticOrder,
  isValidDomain,
  moveAutomaticEntry,
  parseDomainList,
} from "@/lib/web-search";
import { cn } from "@/lib/utils";
import { SearchProviderIcon } from "./search-provider-icon";

const PROVIDER_HINTS: Record<string, string> = {
  auto: "Model provider search when the agent's model supports it, otherwise the next entry of the Automatic order.",
  native: "The model's own search; free search answers for models without one.",
  brave: "Brave Search with a user or session key, else the organization's first Brave account.",
  free: "Keyless best-effort search for every agent.",
  account: "Every search goes to this account first; free search answers when it cannot.",
};

function errorText(error: unknown): string {
  if (error instanceof ApiError && error.message) return error.message;
  return "Could not save the web search defaults. Please try again.";
}

export function SearchDefaultsTab({
  catalog,
  canManage,
}: {
  catalog: SearchProviderCatalog;
  canManage: boolean;
}) {
  const save = useSaveWebSearchDefaults();
  const [draft, setDraft] = useState<WebSearchDefaults>(catalog.defaults);
  const [error, setError] = useState<string | null>(null);
  const [domainError, setDomainError] = useState<string | null>(null);

  const accounts = catalog.accounts;
  const hosted = catalog.built_in.find((provider) => provider.id === AUTOMATIC_HOSTED);
  const entries = automaticEntries(draft, accounts, hosted);
  const busy = !canManage || save.isPending;
  const nativePrice =
    catalog.built_in.find((provider) => provider.id === "native")?.price_note ??
    "Billed by the model provider, about $10 per 1,000 searches";

  const commit = async (next: WebSearchDefaults) => {
    setDraft(next);
    setError(null);
    try {
      await save.mutateAsync(next);
    } catch (err) {
      setError(errorText(err));
      setDraft(catalog.defaults);
    }
  };

  const providerValue =
    draft.provider === "account" && draft.account_id
      ? `account:${draft.account_id}`
      : draft.provider;
  const providerChoices: { value: string; label: string }[] = [
    { value: "auto", label: "Automatic" },
    { value: "native", label: "Model provider search" },
    ...accounts.map((account) => ({
      value: `account:${account.id}`,
      label: accountLabel(account),
    })),
    { value: "free", label: "Free search" },
  ];
  // Keep a stored choice visible even when it is not one of the usual ones.
  if (!providerChoices.some((choice) => choice.value === providerValue)) {
    providerChoices.push({
      value: providerValue ?? "auto",
      label: "Brave Search (any key)",
    });
  }

  const blocked = draft.blocked_domains ?? [];

  return (
    <div className="flex max-w-4xl flex-col gap-3">
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          Only organization admins change web search defaults.
        </p>
      )}
      <div className="divide-y border bg-card">
        <Row
          label="Default search provider"
          description={`Used by every agent with Web search whose own setting is "Organization default". Running sessions pick up a change from their next search.`}
          note={PROVIDER_HINTS[draft.provider ?? "auto"]}
        >
          <Select
            value={providerValue ?? "auto"}
            onValueChange={(value) => {
              if (!value) return;
              const next: WebSearchDefaults = value.startsWith("account:")
                ? {
                    ...draft,
                    provider: "account",
                    account_id: value.slice("account:".length),
                  }
                : {
                    ...draft,
                    provider: value as WebSearchDefaults["provider"],
                    account_id: null,
                  };
              void commit(next);
            }}
            disabled={busy}
          >
            <SelectTrigger aria-label="Default search provider" className="w-full">
              <SelectValue>
                {providerChoices.find((choice) => choice.value === providerValue)?.label}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {providerChoices.map((choice) => (
                <SelectItem key={choice.value} value={choice.value}>
                  {choice.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Row>

        <Row
          label="Automatic order"
          description="Automatic takes the first entry that can serve the model running the turn. Free search always stays last as the safety net."
        >
          <ol aria-label="Automatic order" className="flex flex-col gap-1.5">
            {entries.map((entry, index) => {
              const fixed = entry.kind === "free";
              const position = entries
                .slice(0, index + 1)
                .filter((candidate) => !candidate.inactive).length;
              return (
                <li
                  key={entry.id}
                  className={cn(
                    "flex items-center gap-2 border bg-background px-2.5 py-1.5 text-sm",
                    entry.inactive && "text-muted-foreground",
                  )}
                >
                  <span className="w-4 shrink-0 text-center font-mono text-xs text-muted-foreground">
                    {entry.inactive ? "–" : position}
                  </span>
                  <SearchProviderIcon
                    kind={entry.kind === "account" ? "account" : entry.kind}
                    provider={accounts.find((account) => account.id === entry.id)?.provider}
                  />
                  <span className="min-w-0 flex-1 truncate">
                    {entry.label}
                    {entry.note && <span className="text-muted-foreground"> · {entry.note}</span>}
                  </span>
                  {entry.kind === "native" && !entry.inactive && (
                    <Badge variant="accent" title={nativePrice}>
                      {NATIVE_PRICE_SHORT}
                    </Badge>
                  )}
                  {entry.kind === "hosted" && hosted?.price_note && (
                    <Badge variant="outline" title={hosted.price_note}>
                      {hosted.price_note}
                    </Badge>
                  )}
                  {fixed ? (
                    <Lock aria-label="Always last" className="size-3.5 text-muted-foreground" />
                  ) : (
                    <span className="flex shrink-0">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="size-6"
                        aria-label={`Move ${entry.label} up`}
                        disabled={busy || index === 0}
                        onClick={() =>
                          void commit({
                            ...draft,
                            automatic_order: automaticOrder(moveAutomaticEntry(entries, index, -1)),
                          })
                        }
                      >
                        <ArrowUp className="size-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="size-6"
                        aria-label={`Move ${entry.label} down`}
                        disabled={busy || index >= entries.length - 2}
                        onClick={() =>
                          void commit({
                            ...draft,
                            automatic_order: automaticOrder(moveAutomaticEntry(entries, index, 1)),
                          })
                        }
                      >
                        <ArrowDown className="size-3.5" />
                      </Button>
                    </span>
                  )}
                </li>
              );
            })}
          </ol>
        </Row>

        <Row
          label="Allow model provider search"
          description={`The model's provider runs these searches itself: ${nativePrice.charAt(0).toLowerCase()}${nativePrice.slice(1)}, on that provider's account (or on your Everruns balance for the Everruns LLM provider). They do not pass through this organization's network allowlist. Turn off to keep every search on your own network path.`}
        >
          <Switch
            aria-label="Allow model provider search"
            checked={draft.allow_native !== false}
            disabled={busy}
            onCheckedChange={(checked) => void commit({ ...draft, allow_native: checked })}
          />
        </Row>

        <Row
          label="When a search provider fails"
          description="A failed or rate-limited search can move on to the next provider in the plan; free search is the last one. Agents can set their own."
        >
          <Select
            value={draft.fallback ?? "next"}
            onValueChange={(value) =>
              value &&
              void commit({
                ...draft,
                fallback: value as WebSearchDefaults["fallback"],
              })
            }
            disabled={busy}
          >
            <SelectTrigger aria-label="When a search provider fails" className="w-full">
              <SelectValue>
                {(draft.fallback ?? "next") === "next"
                  ? "Fall back to the next provider"
                  : "Stop and report the failure"}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="next">Fall back to the next provider</SelectItem>
              <SelectItem value="none">Stop and report the failure</SelectItem>
            </SelectContent>
          </Select>
        </Row>

        <Row
          label="Results per search"
          description="When neither the model nor the agent asks for a number. Agents can ask for fewer. Native search decides its own count."
        >
          <Select
            value={String(draft.results_per_search ?? WEB_SEARCH_DEFAULT_RESULTS)}
            onValueChange={(value) =>
              value && void commit({ ...draft, results_per_search: Number(value) })
            }
            disabled={busy}
          >
            <SelectTrigger aria-label="Results per search" className="w-28">
              <SelectValue>
                {String(draft.results_per_search ?? WEB_SEARCH_DEFAULT_RESULTS)}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {Array.from({ length: WEB_SEARCH_MAX_RESULTS }, (_, i) => i + 1).map((count) => (
                <SelectItem key={count} value={String(count)}>
                  {count}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Row>

        <Row
          label="Blocked domains"
          description="Never returned by any provider, for every agent in this organization. Agents can add more."
          problem={domainError}
        >
          <TagInput
            value={blocked.join(", ")}
            placeholder="Add domain…"
            disabled={busy}
            onChange={(value) => {
              const domains = parseDomainList(value);
              const bad = domains.find((domain) => !isValidDomain(domain));
              if (bad) {
                setDomainError(`"${bad}" is not a bare host name like example.com.`);
                return;
              }
              setDomainError(null);
              void commit({ ...draft, blocked_domains: domains });
            }}
          />
        </Row>
      </div>
    </div>
  );
}

function Row({
  label,
  description,
  note,
  problem,
  children,
}: {
  label: string;
  description: string;
  note?: string;
  problem?: string | null;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-2 px-4 py-3.5">
      <div className="min-w-0 flex-[1_1_280px]">
        <div className="text-sm font-medium">{label}</div>
        <div className="text-xs text-muted-foreground">{description}</div>
      </div>
      <div className="flex min-w-[240px] flex-[0_1_340px] flex-col gap-1.5">
        {children}
        {note && <p className="text-xs text-muted-foreground">{note}</p>}
        {problem && (
          <p role="status" className="text-xs text-warning">
            {problem}
          </p>
        )}
      </div>
    </div>
  );
}
