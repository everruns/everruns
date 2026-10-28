"use client";

// Web search → Providers: one card per way the organization can search.
// Model provider search lists which connected model providers offer it and
// what it costs; each search account shows its key status and its place in
// Automatic; free search is always there. Admins connect, test, rotate and
// disconnect accounts here (knowledge/integrations/web-search.md, UI).

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useState, type ReactNode } from "react";
import { FlaskConical, KeyRound, Plus, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useDeleteOrganizationConnection } from "@/hooks/use-organization-connections";
import { previewSearchPlan } from "@/lib/api/search-providers";
import type {
  Provider,
  SearchAccountSummary,
  SearchProviderCatalog,
  SearchTestOutcome,
} from "@/lib/api/types";
import {
  AUTOMATIC_HOSTED,
  accountLabel,
  automaticEntries,
  formatSearchDuration,
} from "@/lib/web-search";
import { cn } from "@/lib/utils";
import { SearchProviderIcon } from "./search-provider-icon";

/** Model provider types whose drivers run native search; mirrors `NATIVE_WEB_SEARCH_PROVIDER_TYPES`. */
const NATIVE_TYPES: { type: string; label: string }[] = [
  { type: "openai", label: "OpenAI" },
  { type: "anthropic", label: "Anthropic" },
  { type: "gemini", label: "Gemini" },
  { type: "openrouter", label: "OpenRouter" },
  { type: "azure_openai", label: "Azure OpenAI" },
];

const TEST_QUERY = "everruns agent platform";

type Notice = { kind: "success" | "error"; text: string };

function StatusDot({ tone, children }: { tone: "ok" | "warn" | "off"; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
      <span
        aria-hidden="true"
        className={cn(
          "size-1.5 rounded-full",
          tone === "ok" && "bg-success",
          tone === "warn" && "bg-warning",
          tone === "off" && "bg-muted-foreground/50",
        )}
      />
      {children}
    </span>
  );
}

function ProviderCard({
  icon,
  title,
  subtitle,
  status,
  description,
  facts,
  footer,
  actions,
  testId,
}: {
  icon: ReactNode;
  title: string;
  subtitle: string;
  status: ReactNode;
  description: string;
  facts: [string, ReactNode][];
  footer: ReactNode;
  actions?: ReactNode;
  testId?: string;
}) {
  return (
    <Card className="flex flex-col gap-0 p-0" data-testid={testId}>
      <div className="flex flex-1 flex-col gap-3 p-4">
        <div className="flex items-start gap-3">
          <div className="mt-1">{icon}</div>
          <div className="min-w-0 flex-1">
            <div className="flex items-start justify-between gap-2">
              <h3 className="text-base font-semibold leading-snug">{title}</h3>
              {status}
            </div>
            <p className="text-xs text-muted-foreground">{subtitle}</p>
          </div>
        </div>
        <p className="text-sm">{description}</p>
        <dl className="grid grid-cols-[minmax(84px,auto)_1fr] gap-x-4 gap-y-1.5 text-sm">
          {facts.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="min-w-0">{value}</dd>
            </div>
          ))}
        </dl>
      </div>
      <div className="flex min-h-11 items-center gap-2 border-t border-border/60 px-4 py-2 text-sm">
        <div className="min-w-0 flex-1 text-muted-foreground">{footer}</div>
        {actions && <div className="flex shrink-0 items-center gap-1">{actions}</div>}
      </div>
    </Card>
  );
}

function testSummary(outcome: SearchTestOutcome, label: string): Notice {
  if (!outcome.ok) {
    return {
      kind: "error",
      text: `${label} test failed: ${outcome.message ?? "no answer"}.`,
    };
  }
  const first = outcome.results[0];
  return {
    kind: "success",
    text: `${label} answered in ${formatSearchDuration(outcome.duration_ms)} with ${outcome.results.length} results${first ? `, first: ${first.title} (${first.site})` : ""}.`,
  };
}

export function SearchProvidersTab({
  catalog,
  isLoading,
  error,
  modelProviders,
  isAdmin,
  onConnect,
  onRotate,
  onGoToDefaults,
  onNotice,
}: {
  catalog: SearchProviderCatalog | undefined;
  isLoading: boolean;
  error: unknown;
  modelProviders: Provider[];
  isAdmin: boolean;
  onConnect: () => void;
  onRotate: (account: SearchAccountSummary) => void;
  onGoToDefaults: () => void;
  onNotice: (notice: Notice) => void;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const remove = useDeleteOrganizationConnection();
  const [testing, setTesting] = useState<string | null>(null);

  if (isLoading) {
    return (
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {[0, 1, 2].map((key) => (
          <Skeleton key={key} className="h-64" />
        ))}
      </div>
    );
  }
  if (error || !catalog) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {ui("Could not load search providers.")}
      </p>
    );
  }

  const defaults = catalog.defaults;
  const allowNative = defaults.allow_native !== false;
  const hostedBuiltIn = catalog.built_in.find((provider) => provider.id === AUTOMATIC_HOSTED);
  const order = automaticEntries(defaults, catalog.accounts, hostedBuiltIn);
  const nativeTypes = new Set(catalog.native_providers.map((provider) => provider.provider_type));
  const nativePrice =
    catalog.built_in.find((provider) => provider.id === "native")?.price_note ??
    "Billed by the model provider, about $10 per 1,000 searches";
  const freeBuiltIn = catalog.built_in.find((provider) => provider.id === "free");
  const unsupported = modelProviders.filter((provider) => !nativeTypes.has(provider.provider_type));

  const runTest = async (provider: string, label: string) => {
    setTesting(provider);
    try {
      const preview = await previewSearchPlan({
        config: {},
        test: { query: TEST_QUERY, provider },
      });
      if (preview.test) onNotice(testSummary(preview.test, label));
    } catch (err) {
      onNotice({
        kind: "error",
        text: `${label} test failed: ${err instanceof Error ? err.message : "unexpected error"}.`,
      });
    } finally {
      setTesting(null);
    }
  };

  const role = (account: SearchAccountSummary): ReactNode => {
    if (defaults.provider === "account" && defaults.account_id === account.id) {
      return <Badge variant="accent">{ui("Organization default")}</Badge>;
    }
    const position = order.findIndex((entry) => entry.id === account.id);
    if (account.status !== "ready")
      return <span className="text-muted-foreground">{ui("Not used")}</span>;
    return (
      <Badge variant="outline" className="border-info/40 text-info">
        {ui("Automatic #")}
        {position + 1}
      </Badge>
    );
  };

  return (
    <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      <ProviderCard
        testId="native-search-card"
        icon={<SearchProviderIcon kind="native" className="size-5" />}
        title={ui("Model provider search")}
        subtitle="Built in · native"
        status={
          <StatusDot tone={allowNative ? "ok" : "off"}>
            {allowNative ? ui("Allowed") : ui("Turned off")}
          </StatusDot>
        }
        description={ui(
          "The model searches through its own provider and cites what it read. Picked first in Automatic.",
        )}
        facts={[
          [
            "Works with",
            <div key="types" className="flex flex-wrap gap-1">
              {NATIVE_TYPES.filter(
                (entry) =>
                  nativeTypes.has(entry.type) ||
                  ["openai", "anthropic", "gemini", "openrouter"].includes(entry.type),
              ).map((entry) => (
                <Badge
                  key={entry.type}
                  variant={nativeTypes.has(entry.type) ? "success" : "outline"}
                  className={cn(!nativeTypes.has(entry.type) && "text-muted-foreground")}
                >
                  {entry.label}
                  {!nativeTypes.has(entry.type) && " · not connected"}
                </Badge>
              ))}
            </div>,
          ],
          ...(unsupported.length > 0
            ? ([["Not supported", unsupported.map((provider) => provider.name).join(", ")]] as [
                string,
                ReactNode,
              ][])
            : []),
          ["Cost", <strong key="cost">{nativePrice}</strong>],
          ["Network", "Runs at the model provider, outside your network allowlist"],
          // A model without native search, or one that cannot apply the
          // agent's sites, falls through to the next provider of the plan.
          // Gemini grounding also needs web search to be the call's only tool.
          [
            "Models",
            "OpenAI models with web search, Claude 3.7 Sonnet, 3.5 Haiku and 4 or later, Gemini 2.0 Flash or later, and OpenRouter models with native search. Others use the next provider in the plan",
          ],
          [
            "Limits",
            "Gemini searches natively only when web search is the agent's only tool and no sites are allowed or blocked",
          ],
        ]}
        footer={
          modelProviders.length > 0
            ? `${catalog.native_providers.length} of your ${modelProviders.length} model providers support it`
            : "Connect a model provider to use it"
        }
        actions={
          <Button variant="ghost" size="sm" onClick={onGoToDefaults}>
            {ui("Settings")}
          </Button>
        }
      />

      {catalog.accounts.map((account) => {
        const ready = account.status === "ready";
        const label = accountLabel(account);
        const connector = catalog.connectors.find((entry) => entry.provider === account.provider);
        return (
          <ProviderCard
            key={account.id}
            testId="search-account-card"
            icon={
              <SearchProviderIcon kind="account" provider={account.provider} className="size-5" />
            }
            title={connector?.name ?? label}
            subtitle={`Search account · ${account.name}`}
            status={
              <StatusDot tone={ready ? "ok" : "warn"}>
                {ready ? ui("Active") : ui("Needs a key")}
              </StatusDot>
            }
            description={
              connector?.description ?? ui("An organization search account. Searches bill its key.")
            }
            facts={[
              [
                "Credentials",
                ready ? (
                  "Configured"
                ) : (
                  <span key="cred" className="text-warning">
                    {ui("Not set")}
                  </span>
                ),
              ],
              ["Connected", new Date(account.connected_at).toLocaleDateString(displayLocale)],
              ["Cost", "Billed on this key by the provider"],
              ["Role", role(account)],
            ]}
            footer="Every agent of the organization can use it; the key never reaches agents."
            actions={
              isAdmin ? (
                <>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={ui("Test {value0}", { value0: label })}
                    title={ui("Run a test search")}
                    disabled={!ready || testing !== null}
                    onClick={() => void runTest(`account:${account.id}`, label)}
                  >
                    <FlaskConical className="size-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={ui("Rotate key of {value0}", { value0: label })}
                    title={ui("Rotate key")}
                    onClick={() => onRotate(account)}
                  >
                    <KeyRound className="size-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={ui("Disconnect {value0}", { value0: label })}
                    title={ui("Disconnect")}
                    disabled={remove.isPending}
                    onClick={async () => {
                      if (!window.confirm(`Disconnect ${label}? Agents set to it fall back.`)) {
                        return;
                      }
                      try {
                        await remove.mutateAsync(account.id);
                        onNotice({
                          kind: "success",
                          text: `${label} disconnected.`,
                        });
                      } catch {
                        onNotice({
                          kind: "error",
                          text: `Could not disconnect ${label}.`,
                        });
                      }
                    }}
                  >
                    <Trash2 className="size-4" />
                  </Button>
                </>
              ) : undefined
            }
          />
        );
      })}

      {hostedBuiltIn && (
        <ProviderCard
          testId="hosted-search-card"
          icon={<SearchProviderIcon kind="hosted" className="size-5" />}
          title={hostedBuiltIn.name}
          subtitle="Built in · no key"
          status={<StatusDot tone="ok">{ui("Active")}</StatusDot>}
          description={hostedBuiltIn.description}
          facts={[
            ["Cost", hostedBuiltIn.price_note ?? "Included"],
            [
              "Role",
              <Badge key="role" variant="outline">
                {(() => {
                  const position = order.findIndex((entry) => entry.kind === "hosted");
                  return position >= 0 ? `Automatic #${position + 1}` : "Not in Automatic";
                })()}
              </Badge>,
            ],
          ]}
          footer="Always available"
        />
      )}

      <ProviderCard
        testId="free-search-card"
        icon={<SearchProviderIcon kind="free" className="size-5" />}
        title={freeBuiltIn?.name ?? ui("Free search")}
        subtitle="Built in · no key"
        status={<StatusDot tone="ok">{ui("Active")}</StatusDot>}
        description={ui(
          "Best-effort results from DuckDuckGo, with DuckDuckGo Lite and Mojeek as fallbacks. Developer queries also check GitHub, npm and crates.io.",
        )}
        facts={[
          ["Cost", "Free"],
          ["Reliability", "Best effort, can be rate limited"],
          [
            "Role",
            <Badge key="role" variant="outline">
              {ui("Last resort in every plan")}
            </Badge>,
          ],
        ]}
        footer="Always available"
        actions={
          isAdmin ? (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={ui("Test Free search")}
              title={ui("Run a test search")}
              disabled={testing !== null}
              onClick={() => void runTest("free", "Free search")}
            >
              <FlaskConical className="size-4" />
            </Button>
          ) : undefined
        }
      />

      {isAdmin && (
        <button
          type="button"
          onClick={onConnect}
          className="flex min-h-48 flex-col items-center justify-center gap-1.5 border border-dashed bg-card/40 p-6 text-center transition-colors hover:border-muted-foreground/60 hover:bg-card"
        >
          <Plus className="size-5 text-muted-foreground" />
          <span className="text-sm font-medium">{ui("Connect search provider")}</span>
          <span className="text-xs text-muted-foreground">
            {catalog.connectors.map((connector) => connector.name).join(" · ") ||
              ui("No search connectors available")}
          </span>
        </button>
      )}
    </div>
  );
}
