"use client";

// Connect provider: one sheet for connect, discover, and choose models.
//
// Connecting a provider, syncing it, and picking which of its models agents may
// use is one task, so the sheet carries it end to end:
//   1. pick a driver (with how many of it are already connected),
//   2. name it and paste credentials (several instances of one driver are
//      allowed, so the name is what tells them apart in model pickers),
//   3. choose models from what the provider just listed, recommended ones
//      already ticked.
// Creating a provider with a credential discovers its models server-side
// before the request returns, which is what makes step 3 possible.
//
// Drivers that sign in instead of taking a key (ChatGPT, OpenRouter's OAuth)
// leave the page for the provider's consent screen; their models are reviewed
// from the "new models" prompt when the browser comes back.

import { useEffect, useMemo, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, ExternalLink } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  ProviderIcon,
  getProviderDescription,
  getProviderLabel,
} from "@/components/providers/provider-icon";
import {
  ModelSelectionList,
  initialSelection,
  selectionActionLabel,
} from "@/components/models/model-selection";
import {
  useCreateProvider,
  useModels,
  useProvidersConfig,
  useReviewProviderModels,
  useSetModelsEnabled,
} from "@/hooks/use-providers";
import { startChatGptLogin } from "@/lib/api/chatgpt";
import { ApiError } from "@/lib/api/client";
import { providerOAuthAuthorizeUrl } from "@/lib/api/providers";
import { isDriverOffered } from "@/lib/api/provider-driver-types";
import type { DriverId, Provider } from "@/lib/api/types";
import { selectionChanges } from "@/lib/model-selection";
import {
  CredentialFields,
  defaultCredentialValues,
  nonEmptyCredentials,
} from "./credential-fields";
import { CredentialCheckStatus } from "./credential-check-status";
import {
  BASE_URL_REQUIRED,
  PROVIDER_TYPES,
  getBaseUrlPlaceholder,
  useDriverCredentialSchema,
} from "./provider-dialogs";
import { useCredentialCheck } from "./use-credential-check";

// The order drivers are listed in: the ones most orgs connect first, then the
// rest of the catalog.
const CONNECT_ORDER: DriverId[] = [
  "anthropic",
  "openai",
  "chatgpt",
  "gemini",
  "openrouter",
  "mistral",
  "meta",
  ...PROVIDER_TYPES,
];

// Where to mint a key, so the form is self-sufficient for a first-time user.
const KEY_CONSOLES: Partial<Record<DriverId, { label: string; href: string }>> = {
  anthropic: {
    label: "console.anthropic.com",
    href: "https://console.anthropic.com/settings/keys",
  },
  openai: { label: "platform.openai.com", href: "https://platform.openai.com/api-keys" },
  gemini: { label: "aistudio.google.com", href: "https://aistudio.google.com/apikey" },
  openrouter: { label: "openrouter.ai/keys", href: "https://openrouter.ai/keys" },
  mistral: { label: "console.mistral.ai", href: "https://console.mistral.ai/api-keys" },
  meta: { label: "llama.developer.meta.com", href: "https://llama.developer.meta.com/api-keys" },
};

// Driver labels double as the default provider name, so the catalog label is
// shortened where it carries a disambiguator nobody needs in a name.
const CONNECT_LABELS: Partial<Record<DriverId, string>> = {
  openai: "OpenAI",
};

export function connectLabel(driver: DriverId): string {
  return CONNECT_LABELS[driver] ?? getProviderLabel(driver);
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

/** Names are unique per org, compared without case or surrounding spaces. */
export function isProviderNameTaken(name: string, providers: Provider[]): boolean {
  const wanted = name.trim().toLowerCase();
  return providers.some((provider) => provider.name.trim().toLowerCase() === wanted);
}

function connectMethod(driver: DriverId, supportsOAuth: boolean): string {
  if (driver === "chatgpt") return "Sign in";
  if (supportsOAuth) return "Key or sign-in";
  if (driver === "bedrock") return "Access keys";
  if (BASE_URL_REQUIRED.includes(driver)) return "Endpoint + key";
  return "API key";
}

type Step =
  | { kind: "list" }
  | { kind: "form"; driver: DriverId }
  | { kind: "models"; provider: Provider };

export function ConnectProviderSheet({
  open,
  onOpenChange,
  providers,
  onConnected,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  providers: Provider[];
  /** Called once the sheet finishes, with a one-line summary for the page. */
  onConnected: (summary: string) => void;
}) {
  const [step, setStep] = useState<Step>({ kind: "list" });

  // Each open starts at the driver list.
  useEffect(() => {
    if (open) setStep({ kind: "list" });
  }, [open]);

  const title =
    step.kind === "list"
      ? "Connect provider"
      : step.kind === "form"
        ? `Connect ${connectLabel(step.driver)}`
        : `Choose ${step.provider.name} models`;

  return (
    <Drawer open={open} onOpenChange={onOpenChange}>
      <DrawerContent className="sm:max-w-xl">
        <DrawerHeader className="flex-row items-center gap-2 p-0 pr-8">
          {step.kind === "form" && (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Back"
              onClick={() => setStep({ kind: "list" })}
            >
              <ArrowLeft className="icon-sharp size-4" />
            </Button>
          )}
          <DrawerTitle>{title}</DrawerTitle>
        </DrawerHeader>
        {open && step.kind === "list" && (
          <DriverList
            providers={providers}
            onPick={(driver) => setStep({ kind: "form", driver })}
          />
        )}
        {open && step.kind === "form" && (
          <ConnectForm
            key={step.driver}
            driver={step.driver}
            providers={providers}
            onCancel={() => onOpenChange(false)}
            onCreated={(provider) => setStep({ kind: "models", provider })}
          />
        )}
        {open && step.kind === "models" && (
          <ChooseModelsStep
            provider={step.provider}
            onDone={(summary) => {
              onConnected(summary);
              onOpenChange(false);
            }}
          />
        )}
      </DrawerContent>
    </Drawer>
  );
}

function DriverList({
  providers,
  onPick,
}: {
  providers: Provider[];
  onPick: (driver: DriverId) => void;
}) {
  const { data: config } = useProvidersConfig();
  const drivers = useMemo(
    () => [...new Set(CONNECT_ORDER)].filter((driver) => isDriverOffered(driver, config?.drivers)),
    [config?.drivers],
  );
  return (
    <>
      <DrawerDescription>
        Several providers of one kind can be connected, for example two Azure resources.
      </DrawerDescription>
      <ul className="-mx-6 min-h-0 flex-1 divide-y overflow-y-auto border-y">
        {drivers.map((driver) => {
          const connected = providers.filter((p) => p.provider_type === driver).length;
          const supportsOAuth =
            config?.drivers?.find((d) => d.driver === driver)?.supports_oauth ??
            driver === "openrouter";
          return (
            <li key={driver}>
              <button
                type="button"
                onClick={() => onPick(driver)}
                className="flex w-full items-center gap-3 px-6 py-3 text-left transition-colors hover:bg-muted focus-visible:bg-muted focus-visible:outline-none"
              >
                <ProviderIcon providerType={driver} size="sm" showBackground={false} />
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="text-sm font-medium">{connectLabel(driver)}</span>
                  <span className="truncate text-xs text-muted-foreground">
                    {getProviderDescription(driver)}
                  </span>
                </span>
                {connected > 0 && <Badge variant="success">{connected} connected</Badge>}
                <span className="whitespace-nowrap text-xs text-muted-foreground">
                  {connected > 0 ? "Add another" : connectMethod(driver, supportsOAuth)}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </>
  );
}

function errorText(error: unknown, fallback: string): string {
  if (error instanceof ApiError && error.message) return error.message;
  return fallback;
}

function ConnectForm({
  driver,
  providers,
  onCancel,
  onCreated,
}: {
  driver: DriverId;
  providers: Provider[];
  onCancel: () => void;
  onCreated: (provider: Provider) => void;
}) {
  const router = useRouter();
  const createProvider = useCreateProvider();
  const { schema, supportsOAuth } = useDriverCredentialSchema(driver);
  const existing = providers.filter((provider) => provider.provider_type === driver);
  const label = connectLabel(driver);
  const [name, setName] = useState(() =>
    uniqueProviderName(
      label,
      providers.map((p) => p.name),
    ),
  );
  const baseUrlRequired = BASE_URL_REQUIRED.includes(driver);
  const [customEndpoint, setCustomEndpoint] = useState(baseUrlRequired);
  const [baseUrl, setBaseUrl] = useState("");
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [redirecting, setRedirecting] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const signInOnly = driver === "chatgpt";

  useEffect(() => {
    setCredentials(defaultCredentialValues(schema));
  }, [schema]);

  const { state: check } = useCredentialCheck({
    providerType: driver,
    credentials,
    baseUrl: customEndpoint ? baseUrl : undefined,
    enabled: !signInOnly,
  });

  const trimmedName = name.trim();
  const duplicate = trimmedName !== "" && isProviderNameTaken(trimmedName, providers);
  const submitted = nonEmptyCredentials(credentials);
  const hasCredential = Object.keys(submitted).length > 0;
  const endpoint = customEndpoint ? baseUrl.trim() : "";
  const formReady =
    trimmedName !== "" && !duplicate && (!baseUrlRequired || endpoint !== "") && !redirecting;
  const keyConsole = KEY_CONSOLES[driver];

  const create = () =>
    createProvider.mutateAsync({
      name: trimmedName,
      provider_type: driver,
      base_url: endpoint || undefined,
      credentials: hasCredential ? submitted : undefined,
    });

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!formReady || !hasCredential) return;
    setError(null);
    try {
      onCreated(await create());
    } catch (err) {
      setError(errorText(err, `Could not connect ${label}. Please try again.`));
    }
  };

  // Sign-in drivers need the provider row first: the consent flow binds its
  // state to the provider id.
  const handleSignIn = async () => {
    if (!formReady) return;
    setError(null);
    setRedirecting(true);
    const popup = driver === "chatgpt" ? window.open("about:blank", "_blank") : null;
    if (popup) popup.opener = null;
    try {
      const provider = await create();
      if (driver === "chatgpt") {
        const { authorize_url } = await startChatGptLogin(provider.id);
        router.push(`/models/providers/${provider.id}`);
        if (popup) popup.location.assign(authorize_url);
        else window.location.assign(authorize_url);
      } else {
        window.location.assign(providerOAuthAuthorizeUrl(provider.id));
      }
    } catch (err) {
      popup?.close();
      setRedirecting(false);
      setError(errorText(err, `Could not start the ${label} sign-in. Please try again.`));
    }
  };

  return (
    <form onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="-mx-6 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6">
        {existing.length > 0 && (
          <div className="flex flex-col gap-1.5 border bg-muted/40 p-3 text-xs">
            <span className="font-medium">Already connected</span>
            {existing.map((provider) => (
              <span key={provider.id} className="flex justify-between gap-3">
                <span className="font-medium">{provider.name}</span>
                <span className="truncate font-mono text-muted-foreground">
                  {provider.base_url ?? "Driver default"}
                </span>
              </span>
            ))}
            <span className="text-muted-foreground">
              This adds another {label} provider with its own credentials and models. Give it a name
              that tells them apart in model pickers.
            </span>
          </div>
        )}

        <div className="space-y-1.5">
          <Label htmlFor="connect-provider-name">Name</Label>
          <Input
            id="connect-provider-name"
            ref={nameRef}
            value={name}
            onChange={(event) => setName(event.target.value)}
            aria-invalid={duplicate || undefined}
            aria-describedby="connect-provider-name-hint"
          />
          <p
            id="connect-provider-name-hint"
            className={duplicate ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
          >
            {duplicate
              ? "A provider with this name already exists. Names must be unique."
              : "Shown in model pickers as “Model (Name)”."}
          </p>
        </div>

        {signInOnly ? (
          <p className="text-sm text-muted-foreground">
            Uses your ChatGPT plan through sign-in, with no API key. It is personal to you: other
            members do not see it, and shared defaults and Playground skip it.
          </p>
        ) : (
          <>
            {baseUrlRequired || customEndpoint ? (
              <div className="space-y-1.5">
                <Label htmlFor="connect-provider-endpoint">
                  Endpoint{baseUrlRequired ? "" : " (optional)"}
                </Label>
                <Input
                  id="connect-provider-endpoint"
                  className="font-mono"
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                  placeholder={getBaseUrlPlaceholder(driver)}
                />
                {driver === "azure_openai" && (
                  <p className="text-xs text-muted-foreground">
                    Your Azure OpenAI `.../openai/v1` endpoint on `openai.azure.com` or
                    `services.ai.azure.com`.
                  </p>
                )}
              </div>
            ) : (
              <button
                type="button"
                className="self-start text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                onClick={() => setCustomEndpoint(true)}
              >
                Use a custom endpoint
              </button>
            )}

            {schema && (
              <div className="space-y-2">
                <CredentialFields
                  schema={schema}
                  values={credentials}
                  onChange={(key, value) => setCredentials((prev) => ({ ...prev, [key]: value }))}
                  idPrefix="connect-provider"
                  allowEmptySubmit={supportsOAuth}
                />
                <CredentialCheckStatus state={check} />
                {keyConsole && (
                  <p className="text-xs text-muted-foreground">
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
                    . Keys are encrypted at rest.
                  </p>
                )}
              </div>
            )}
          </>
        )}

        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </div>

      <DrawerFooter className="flex-row flex-wrap justify-end gap-2 p-0">
        <Button type="button" variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        {(signInOnly || supportsOAuth) && (
          <Button
            type="button"
            variant={signInOnly ? "default" : "outline"}
            onClick={() => void handleSignIn()}
            disabled={!formReady || createProvider.isPending}
          >
            {redirecting ? "Redirecting…" : `Sign in with ${label}`}
          </Button>
        )}
        {!signInOnly && (
          <Button type="submit" disabled={!formReady || !hasCredential || createProvider.isPending}>
            {createProvider.isPending && !redirecting ? "Connecting…" : "Connect"}
          </Button>
        )}
      </DrawerFooter>
    </form>
  );
}

function ChooseModelsStep({
  provider,
  onDone,
}: {
  provider: Provider;
  onDone: (summary: string) => void;
}) {
  const { data: models = [], isFetching } = useModels();
  const setModelsEnabled = useSetModelsEnabled();
  const review = useReviewProviderModels();
  const [error, setError] = useState<string | null>(null);
  const discovered = useMemo(
    () => models.filter((model) => model.provider_id === provider.id && !model.stale),
    [models, provider.id],
  );
  const [selected, setSelected] = useState<Set<string> | null>(null);

  // Seed once the provider's models arrive (the create invalidated the list).
  useEffect(() => {
    if (selected === null && !isFetching && discovered.length > 0) {
      setSelected(initialSelection(discovered, { recommend: true }));
    }
  }, [selected, isFetching, discovered]);

  const finish = async () => {
    setError(null);
    const changes = selected ? selectionChanges(discovered, selected) : [];
    const { failed } = await setModelsEnabled.mutateAsync(changes);
    if (failed.length > 0) {
      setError(`${failed.length} of ${changes.length} models could not be updated. Try again.`);
      return;
    }
    await review.mutateAsync([provider.id]).catch(() => undefined);
    const enabled = discovered.filter((model) => selected?.has(model.id)).length;
    onDone(
      enabled > 0
        ? `${provider.name} connected. ${enabled} ${enabled === 1 ? "model" : "models"} enabled.`
        : `${provider.name} connected.`,
    );
  };

  if (isFetching && selected === null) {
    return <p className="text-sm text-muted-foreground">Discovering models…</p>;
  }

  if (discovered.length === 0) {
    return (
      <>
        <DrawerDescription>
          {provider.name} is connected, but it listed no models yet. Sync it from its card once the
          account has access, or add a model by hand.
        </DrawerDescription>
        <DrawerFooter className="flex-row justify-end p-0">
          <Button onClick={() => onDone(`${provider.name} connected.`)}>Done</Button>
        </DrawerFooter>
      </>
    );
  }

  const busy = setModelsEnabled.isPending || review.isPending;
  return (
    <>
      <DrawerDescription>
        {provider.name} lists {discovered.length} {discovered.length === 1 ? "model" : "models"}.
        Recommended ones are ticked: the newest of each family and calibrated decision models.
      </DrawerDescription>
      <ModelSelectionList
        models={discovered}
        selected={selected ?? new Set()}
        onSelectedChange={setSelected}
      />
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <DrawerFooter className="flex-row justify-end gap-2 p-0">
        <Button onClick={() => void finish()} disabled={busy}>
          {busy ? "Saving…" : selectionActionLabel(discovered, selected ?? new Set(), "enable")}
        </Button>
      </DrawerFooter>
    </>
  );
}
