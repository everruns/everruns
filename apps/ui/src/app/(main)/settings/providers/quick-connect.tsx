"use client";

// Quick-connect grid: the fast path onto the providers page.
//
// The full Add-Provider dialog asks for a name, a driver, a base URL and a
// credential before anything happens — four decisions for the five providers
// almost everyone actually connects. This section reduces those to one: pick a
// tile, paste a key. The name is derived (deduped per driver), the driver comes
// from the tile, and the base URL stays at the driver default. Providers that
// genuinely need more than a key (Azure, Bedrock, custom endpoints) keep the
// dialog, reached through the "Another provider" tile.
//
// Drivers that declare OAuth (today OpenRouter) offer both methods on the tile:
// "Sign in" creates the provider and hands the browser to the driver's authorize
// endpoint — the flow needs a provider id to bind its state cookie to, so
// creation has to happen first — and "API key" opens the same paste panel as
// every other tile.
//
// The key is verified as it is typed (see `useCredentialCheck`); a rejection is
// shown but never blocks saving, because a key can be valid for an org the
// probe cannot see, and an unverified provider is still better than a lost one.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { IconTile } from "@/components/layout/page-layout";
import { Check, ExternalLink, Plus, SlidersHorizontal, X } from "lucide-react";
import {
  ProviderIcon,
  getProviderLabel,
  getProviderDescription,
} from "@/components/providers/provider-icon";
import { useCreateProvider, useProvidersConfig } from "@/hooks/use-providers";
import { providerOAuthAuthorizeUrl, providerSupportsOAuth } from "@/lib/api/providers";
import type { DriverId, Provider } from "@/lib/api/types";
import { useCredentialCheck } from "./use-credential-check";
import { CredentialCheckStatus } from "./credential-check-status";

// The drivers that need nothing but a key, in the order they are offered.
const QUICK_CONNECT_DRIVERS: DriverId[] = ["anthropic", "openai", "gemini", "openrouter", "meta"];

// Shape hints so a pasted key can be eyeballed against the expected prefix.
const KEY_PLACEHOLDERS: Partial<Record<DriverId, string>> = {
  anthropic: "sk-ant-api03-…",
  openai: "sk-proj-…",
  gemini: "AIza…",
  openrouter: "sk-or-v1-…",
  meta: "LLM|…",
};

// Where to mint a key, so the panel is self-sufficient for a first-time user.
const KEY_CONSOLES: Partial<Record<DriverId, { label: string; href: string }>> = {
  anthropic: {
    label: "console.anthropic.com",
    href: "https://console.anthropic.com/settings/keys",
  },
  openai: { label: "platform.openai.com", href: "https://platform.openai.com/api-keys" },
  gemini: { label: "aistudio.google.com", href: "https://aistudio.google.com/apikey" },
  openrouter: { label: "openrouter.ai/keys", href: "https://openrouter.ai/keys" },
  meta: { label: "llama.developer.meta.com", href: "https://llama.developer.meta.com/api-keys" },
};

// Tile titles double as the derived provider name, so the driver's full catalog
// label is shortened where it carries a disambiguator the quick path never needs
// ("OpenAI (Responses)" is the only OpenAI on offer here).
const QUICK_CONNECT_LABELS: Partial<Record<DriverId, string>> = {
  openai: "OpenAI",
};

export function quickConnectLabel(driver: DriverId): string {
  return QUICK_CONNECT_LABELS[driver] ?? getProviderLabel(driver);
}

/** Derive a provider name that does not collide with one already connected. */
export function uniqueProviderName(base: string, taken: string[]): string {
  const existing = new Set(taken.map((n) => n.trim().toLowerCase()));
  if (!existing.has(base.toLowerCase())) return base;
  for (let n = 2; ; n += 1) {
    const candidate = `${base} ${n}`;
    if (!existing.has(candidate.toLowerCase())) return candidate;
  }
}

export function QuickConnect({
  providers,
  onOpenAdvanced,
}: {
  providers: Provider[];
  onOpenAdvanced: () => void;
}) {
  const { data: config } = useProvidersConfig();
  const createProvider = useCreateProvider();
  // The driver whose paste panel is open, and the driver currently being handed
  // to an OAuth redirect (its tile shows a busy label until navigation happens).
  const [openDriver, setOpenDriver] = useState<DriverId | null>(null);
  const [oauthDriver, setOauthDriver] = useState<DriverId | null>(null);
  const [error, setError] = useState<string | null>(null);

  const supportsOAuth = (driver: DriverId): boolean =>
    config?.drivers.find((d) => d.driver === driver)?.supports_oauth ??
    providerSupportsOAuth(driver);

  const connectedCount = (driver: DriverId): number =>
    providers.filter((p) => p.provider_type === driver).length;

  const startOAuth = async (driver: DriverId) => {
    setError(null);
    setOpenDriver(null);
    setOauthDriver(driver);
    try {
      const provider = await createProvider.mutateAsync({
        name: uniqueProviderName(
          quickConnectLabel(driver),
          providers.map((p) => p.name),
        ),
        provider_type: driver,
      });
      window.location.href = providerOAuthAuthorizeUrl(provider.id);
    } catch {
      setOauthDriver(null);
      setError(`Could not start the ${quickConnectLabel(driver)} sign-in. Please try again.`);
    }
  };

  return (
    <section>
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-3">
        <h3 className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
          Connect
        </h3>
        <p className="text-xs text-muted-foreground">
          Paste a key and it verifies itself. No name needed.
        </p>
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6">
        {QUICK_CONNECT_DRIVERS.map((driver) => (
          <QuickConnectTile
            key={driver}
            driver={driver}
            connected={connectedCount(driver)}
            dualAuth={supportsOAuth(driver)}
            oauthPending={oauthDriver === driver}
            onKeyConnect={() => {
              setError(null);
              setOpenDriver(driver);
            }}
            onOAuthConnect={() => void startOAuth(driver)}
          />
        ))}

        <div className="flex flex-col gap-2.5 border border-dashed p-4">
          <div className="flex items-center gap-2.5">
            <IconTile size="md" icon={<SlidersHorizontal className="icon-sharp size-4" />} />
            <span className="text-sm font-semibold">Another provider</span>
          </div>
          <p className="flex-1 text-xs leading-relaxed text-muted-foreground">
            Azure OpenAI, AWS Bedrock, Fireworks, or any OpenAI-compatible endpoint.
          </p>
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">Needs more setup</span>
            <Button variant="outline" size="sm" className="ml-auto" onClick={onOpenAdvanced}>
              Choose type
            </Button>
          </div>
        </div>
      </div>

      {error ? <p className="mt-3 text-sm text-destructive">{error}</p> : null}

      {openDriver ? (
        <QuickConnectPanel
          key={openDriver}
          driver={openDriver}
          existingNames={providers.map((p) => p.name)}
          onClose={() => setOpenDriver(null)}
        />
      ) : null}
    </section>
  );
}

function QuickConnectTile({
  driver,
  connected,
  dualAuth,
  oauthPending,
  onKeyConnect,
  onOAuthConnect,
}: {
  driver: DriverId;
  connected: number;
  dualAuth: boolean;
  oauthPending: boolean;
  onKeyConnect: () => void;
  onOAuthConnect: () => void;
}) {
  const label = quickConnectLabel(driver);
  return (
    <div className="flex flex-col gap-2.5 border bg-background p-4">
      <div className="flex items-center gap-2.5">
        <IconTile
          size="md"
          icon={
            <ProviderIcon providerType={driver} size="sm" showBackground={false} className="p-0" />
          }
        />
        <span className="min-w-0 truncate text-sm font-semibold" title={label}>
          {label}
        </span>
      </div>
      <p className="flex-1 text-xs leading-relaxed text-muted-foreground">
        {getProviderDescription(driver)}
      </p>
      {connected > 0 ? (
        <Badge variant="success" className="self-start">
          <Check className="icon-sharp size-3" />
          {connected} connected
        </Badge>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        {dualAuth ? (
          <>
            <span className="text-xs text-muted-foreground">Key or sign-in</span>
            <Button
              variant="outline"
              size="sm"
              className="ml-auto"
              onClick={onOAuthConnect}
              disabled={oauthPending}
            >
              {oauthPending ? "Redirecting…" : "Sign in"}
            </Button>
            <Button variant="outline" size="sm" onClick={onKeyConnect}>
              API key
            </Button>
          </>
        ) : (
          <>
            <span className="text-xs text-muted-foreground">API key</span>
            <Button variant="outline" size="sm" className="ml-auto" onClick={onKeyConnect}>
              <Plus className="icon-sharp size-3.5" />
              {connected > 0 ? "Add another" : "Connect"}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

function QuickConnectPanel({
  driver,
  existingNames,
  onClose,
}: {
  driver: DriverId;
  existingNames: string[];
  onClose: () => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const createProvider = useCreateProvider();

  const label = quickConnectLabel(driver);
  const name = uniqueProviderName(label, existingNames);
  const keyConsole = KEY_CONSOLES[driver];

  const { state: check, recheck } = useCredentialCheck({
    providerType: driver,
    credentials: { api_key: apiKey },
  });

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!apiKey.trim()) return;
    setSaveError(null);
    try {
      await createProvider.mutateAsync({
        name,
        provider_type: driver,
        api_key: apiKey,
      });
      onClose();
    } catch {
      setSaveError("Failed to add the provider. Please try again.");
    }
  };

  return (
    <form onSubmit={handleSubmit} className="mt-3 border bg-card">
      <div className="flex items-start gap-3 border-b p-4">
        <IconTile
          size="lg"
          icon={
            <ProviderIcon providerType={driver} size="md" showBackground={false} className="p-0" />
          }
        />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-base font-semibold">Connect {label}</span>
            <Badge variant="outline" className="font-mono">
              {name}
            </Badge>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Named automatically. Rename it later from the provider page.
          </p>
        </div>
        <Button type="button" variant="ghost" size="icon-sm" aria-label="Close" onClick={onClose}>
          <X className="icon-sharp size-4" />
        </Button>
      </div>

      <div className="space-y-2 p-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <Label htmlFor="quick-connect-api-key">API key</Label>
          <CredentialCheckStatus state={check} />
        </div>
        <Input
          id="quick-connect-api-key"
          ref={inputRef}
          type="password"
          autoComplete="off"
          spellCheck={false}
          className="max-w-xl font-mono"
          placeholder={KEY_PLACEHOLDERS[driver] ?? "sk-…"}
          value={apiKey}
          onChange={(e: React.ChangeEvent<HTMLInputElement>) => setApiKey(e.target.value)}
        />
        {check.status === "rejected" ? (
          <p className="max-w-xl text-xs leading-relaxed text-muted-foreground">
            <span className="font-medium text-destructive">{label} rejected this key.</span> It may
            be revoked, scoped to another organization, or truncated on copy. You can still add it
            and fix the key later.
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">
            {keyConsole ? (
              <>
                Generate one at{" "}
                <a
                  href={keyConsole.href}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-1 text-foreground hover:underline"
                >
                  {keyConsole.label}
                  <ExternalLink className="icon-sharp size-3" />
                </a>
                .{" "}
              </>
            ) : null}
            Verification runs as you type — there is nothing to click.
          </p>
        )}
        {saveError ? <p className="text-sm text-destructive">{saveError}</p> : null}
      </div>

      <div className="flex flex-wrap items-center gap-2 border-t bg-muted/40 p-3">
        <p className="mr-auto text-xs text-muted-foreground">
          Keys are encrypted at rest. Models are discovered on connect.
        </p>
        <Button type="button" variant="outline" onClick={onClose}>
          Cancel
        </Button>
        {check.status === "rejected" ? (
          <Button type="button" variant="outline" onClick={recheck}>
            Check again
          </Button>
        ) : null}
        <Button type="submit" disabled={!apiKey.trim() || createProvider.isPending}>
          {createProvider.isPending
            ? "Adding…"
            : check.status === "rejected"
              ? `Add ${label} anyway`
              : `Add ${label}`}
        </Button>
      </div>
    </form>
  );
}
