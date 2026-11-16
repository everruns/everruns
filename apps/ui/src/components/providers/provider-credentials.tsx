"use client";

// Provider credentials: how a provider is connected, shared by the connect
// sheet and the key button on a provider card.
//
// Drivers that support sign-in offer two ways to get a credential: sign in
// with the provider, which mints a key in the user's account, or paste a key
// they already have. Both end in the same stored credential, so they are one
// choice ("How to connect") with one primary button that follows it, instead
// of a sign-in button and a key form side by side. Drivers without sign-in
// skip the choice and show the key fields alone.
//
// Adding and replacing a credential use the same drawer pattern; the card's
// key button opens `ProviderCredentialsSheet`, worded for replacing. Whether
// the current credential came from sign-in or a pasted key is not recorded
// server-side, so the status block shows only that one is set and when the
// provider last changed.

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useEffect, useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
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
import { getProviderLabel } from "@/components/providers/provider-icon";
import { useUpdateProvider } from "@/hooks/use-providers";
import { ApiError } from "@/lib/api/client";
import { startProviderOAuth } from "@/lib/api/providers";
import type { DriverId, Provider } from "@/lib/api/types";
import { formatDate } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import {
  CredentialFields,
  defaultCredentialValues,
  nonEmptyCredentials,
} from "./credential-fields";
import { CredentialCheckStatus } from "./credential-check-status";
import { useDriverCredentialSchema } from "./provider-drivers";
import { useCredentialCheck, type CredentialCheckState } from "./use-credential-check";

export type ConnectMethod = "signin" | "key";

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

export function KeyConsoleHint({ driver }: { driver: DriverId }) {
  const ui = useUiTranslations();
  const keyConsole = KEY_CONSOLES[driver];
  if (!keyConsole) return null;
  return (
    <p className="text-xs text-muted-foreground">
      {ui("Generate one at")}{" "}
      <a
        href={keyConsole.href}
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-1 text-foreground hover:underline"
      >
        {keyConsole.label}
        <ExternalLink className="icon-sharp size-3" />
      </a>
      {ui(". Keys are encrypted at rest.")}
    </p>
  );
}

/** A key may be saved unless the live check is still running or rejected it. */
export function keyCheckAllowsSave(check: CredentialCheckState): boolean {
  return check.status !== "checking" && check.status !== "rejected";
}

function MethodOption({
  name,
  value,
  checked,
  onSelect,
  title,
  description,
  recommended,
  children,
}: {
  name: string;
  value: ConnectMethod;
  checked: boolean;
  onSelect: (value: ConnectMethod) => void;
  title: string;
  description: string;
  recommended?: boolean;
  children?: ReactNode;
}) {
  const ui = useUiTranslations();
  return (
    <div
      className={cn(
        "border transition-colors",
        checked ? "border-foreground/60 bg-muted/30" : "hover:bg-muted/40",
      )}
    >
      <label className="flex cursor-pointer items-start gap-3 p-3">
        <input
          type="radio"
          name={name}
          value={value}
          checked={checked}
          onChange={() => onSelect(value)}
          className="mt-0.5 size-4 accent-foreground"
        />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="flex flex-wrap items-center gap-2 text-sm font-medium">
            {title}
            {recommended && <Badge variant="secondary">{ui("Recommended")}</Badge>}
          </span>
          <span className="text-xs text-muted-foreground">{description}</span>
        </span>
      </label>
      {checked && children && <div className="space-y-2 px-3 pb-3 pl-10">{children}</div>}
    </div>
  );
}

/**
 * The "How to connect" choice for drivers that support sign-in. `keyFields`
 * renders under the key option only while it is selected.
 */
export function ConnectMethodOptions({
  label,
  method,
  onMethodChange,
  replacing = false,
  keyFields,
  idPrefix,
}: {
  label: string;
  method: ConnectMethod;
  onMethodChange: (method: ConnectMethod) => void;
  /** Wording for replacing an existing credential instead of adding one. */
  replacing?: boolean;
  keyFields: ReactNode;
  idPrefix: string;
}) {
  const ui = useUiTranslations();
  const name = `${idPrefix}-method`;
  return (
    <fieldset className="flex flex-col gap-2.5">
      <legend className="mb-2.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {replacing ? ui("Replace credential") : ui("How to connect")}
      </legend>
      <MethodOption
        name={name}
        value="signin"
        checked={method === "signin"}
        onSelect={onMethodChange}
        recommended
        title={
          replacing
            ? ui("Sign in again with {label}", { label })
            : ui("Sign in with {label}", { label })
        }
        description={
          replacing
            ? ui(
                "Creates a fresh key in your {label} account and replaces the current one. The old key keeps working until you revoke it in {label}.",
                { label },
              )
            : ui(
                "Approve access in {label}. It creates a key for this provider, and Everruns stores it encrypted. Revoke it any time in your {label} account.",
                { label },
              )
        }
      >
        <p className="text-xs text-muted-foreground">
          {ui("You leave Everruns for a moment and come back here when you approve.")}
        </p>
      </MethodOption>
      <MethodOption
        name={name}
        value="key"
        checked={method === "key"}
        onSelect={onMethodChange}
        title={replacing ? ui("Paste a different API key") : ui("Paste an API key")}
        description={
          replacing
            ? ui("Replaces the current credential with a key you created.")
            : ui("Use a key you already created in {label}.", { label })
        }
      >
        {keyFields}
      </MethodOption>
    </fieldset>
  );
}

/** Primary button text for handing off to the provider's sign-in. */
export function ContinueToProviderLabel({ label }: { label: string }) {
  const ui = useUiTranslations();
  return (
    <>
      {ui("Continue to {label}", { label })}
      <ExternalLink className="icon-sharp size-3.5" />
    </>
  );
}

/**
 * The provider card's key button: shows the current credential and replaces
 * it, by signing in again or pasting a different key.
 */
export function ProviderCredentialsSheet({
  provider,
  open,
  onOpenChange,
  onSaved,
}: {
  provider: Provider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called after a pasted key is saved, with a one-line summary for the page. */
  onSaved?: (summary: string) => void;
}) {
  const ui = useUiTranslations();
  return (
    <Drawer open={open} onOpenChange={onOpenChange}>
      <DrawerContent className="sm:max-w-xl" chatProtected="Provider credentials">
        <DrawerHeader className="p-0 pr-8">
          <DrawerTitle>
            {provider ? ui("{name} credentials", { name: provider.name }) : ui("Credentials")}
          </DrawerTitle>
        </DrawerHeader>
        {open && provider && (
          <CredentialsForm
            key={provider.id}
            provider={provider}
            onClose={() => onOpenChange(false)}
            onSaved={onSaved}
          />
        )}
      </DrawerContent>
    </Drawer>
  );
}

function CredentialsForm({
  provider,
  onClose,
  onSaved,
}: {
  provider: Provider;
  onClose: () => void;
  onSaved?: (summary: string) => void;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const { schema, supportsOAuth } = useDriverCredentialSchema(provider.provider_type);
  const updateProvider = useUpdateProvider(provider.id);
  const label = getProviderLabel(provider.provider_type);
  const [method, setMethod] = useState<ConnectMethod>(supportsOAuth ? "signin" : "key");
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [redirecting, setRedirecting] = useState(false);
  const usingKey = !supportsOAuth || method === "key";

  // Secrets are write-only, so existing values are never pre-filled.
  useEffect(() => {
    setCredentials(defaultCredentialValues(schema));
  }, [schema]);

  // A replacement key is checked before it overwrites a working one.
  const { state: check } = useCredentialCheck({
    providerType: provider.provider_type,
    credentials,
    baseUrl: provider.base_url,
    enabled: usingKey,
  });

  const submitted = nonEmptyCredentials(credentials);
  const hasInput = Object.keys(submitted).length > 0;
  const canSave = hasInput && keyCheckAllowsSave(check) && !updateProvider.isPending;

  const handleSignIn = () => {
    setError(null);
    setRedirecting(true);
    startProviderOAuth(provider.id).catch((err) => {
      setRedirecting(false);
      setError(
        err instanceof ApiError && err.message
          ? err.message
          : ui("Could not start the {label} sign-in. Please try again.", { label }),
      );
    });
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!usingKey) {
      handleSignIn();
      return;
    }
    if (!canSave) return;
    setError(null);
    try {
      await updateProvider.mutateAsync({
        provider_type: provider.provider_type,
        credentials: submitted,
      });
      onSaved?.(ui("{name} key saved.", { name: provider.name }));
      onClose();
    } catch (err) {
      setError(
        err instanceof ApiError && err.message
          ? err.message
          : ui("Could not save the key. Please try again."),
      );
    }
  };

  const keyFields = schema ? (
    <div className="space-y-2">
      <CredentialFields
        schema={schema}
        values={credentials}
        onChange={(key, value) => setCredentials((prev) => ({ ...prev, [key]: value }))}
        idPrefix="provider-credentials"
      />
      <CredentialCheckStatus state={check} />
      <KeyConsoleHint driver={provider.provider_type} />
    </div>
  ) : null;

  const connected = provider.api_key_set || provider.managed;

  return (
    <form onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col gap-4">
      <DrawerDescription className="sr-only">
        {ui("Replace the credential this provider uses.")}
      </DrawerDescription>
      <div className="-mx-6 flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-6">
        <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 border bg-muted/40 p-3 text-xs">
          <dt className="text-muted-foreground">{ui("Status")}</dt>
          <dd className="flex items-center gap-1.5">
            <span
              aria-hidden
              className={cn(
                "inline-block size-1.5 rounded-full",
                connected ? "bg-success" : "bg-muted-foreground",
              )}
            />
            {connected ? ui("Connected") : ui("Not connected")}
          </dd>
          <dt className="text-muted-foreground">{ui("Credential")}</dt>
          <dd>
            {provider.managed
              ? ui("Managed by this server")
              : provider.api_key_set
                ? ui("Set, encrypted at rest")
                : ui("None yet")}
          </dd>
          <dt className="text-muted-foreground">{ui("Last changed")}</dt>
          <dd>{formatDate(provider.updated_at, displayLocale)}</dd>
        </dl>

        {supportsOAuth ? (
          <ConnectMethodOptions
            label={label}
            method={method}
            onMethodChange={setMethod}
            replacing={provider.api_key_set}
            keyFields={keyFields}
            idPrefix="provider-credentials"
          />
        ) : (
          keyFields
        )}

        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </div>
      <DrawerFooter className="flex-row justify-end gap-2 p-0">
        <Button type="button" variant="outline" onClick={onClose}>
          {ui("Cancel")}
        </Button>
        {usingKey ? (
          <Button type="submit" disabled={!canSave}>
            {updateProvider.isPending ? ui("Saving…") : ui("Save key")}
          </Button>
        ) : (
          <Button type="submit" disabled={redirecting}>
            {redirecting ? ui("Redirecting…") : <ContinueToProviderLabel label={label} />}
          </Button>
        )}
      </DrawerFooter>
    </form>
  );
}
