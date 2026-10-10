"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useProvidersConfig, useUpdateProvider } from "@/hooks/use-providers";
import { getProviderLabel } from "@/components/providers/provider-icon";
import { providerSupportsOAuth, providerOAuthAuthorizeUrl } from "@/lib/api/providers";
import type { Provider, DriverId, CredentialFormSchema } from "@/lib/api/types";
import {
  CredentialFields,
  defaultCredentialValues,
  nonEmptyCredentials,
} from "./credential-fields";
import { useCredentialCheck } from "./use-credential-check";
import { CredentialCheckStatus } from "./credential-check-status";

// Ordered list of provider types offered by the connect sheet. Labels and descriptions come
// from the centralized `getProviderLabel` / `getProviderDescription` mappings so
// the dropdown and the selected-value trigger never diverge.
export const PROVIDER_TYPES: DriverId[] = [
  "openai",
  "openrouter",
  "azure_openai",
  "openai_completions",
  "anthropic",
  "gemini",
  "bedrock",
  "mai",
  "fireworks",
  "meta",
  "mistral",
];

// Drivers whose endpoint (`base_url`) is mandatory.
export const BASE_URL_REQUIRED: DriverId[] = ["azure_openai", "mai"];

export function getBaseUrlPlaceholder(providerType: DriverId): string {
  switch (providerType) {
    case "azure_openai":
      return "https://your-resource.openai.azure.com/openai/v1";
    case "openai":
      return "https://api.openai.com/v1";
    case "openrouter":
      return "https://openrouter.ai/api/v1";
    case "openai_completions":
      return "https://api.openai.com/v1/chat/completions";
    case "anthropic":
      return "https://api.anthropic.com/v1/messages";
    case "gemini":
      return "https://generativelanguage.googleapis.com";
    case "mai":
      return "https://your-resource.services.ai.azure.com";
    case "fireworks":
      return "https://api.fireworks.ai/inference/v1";
    case "meta":
      return "https://api.meta.ai/v1";
    case "mistral":
      return "https://api.mistral.ai/v1";
    default:
      return "https://api.example.com";
  }
}

// Resolve a driver's declared credential schema (and whether it supports OAuth)
// from the providers config. Falls back to the client OAuth shim when the
// config has not loaded yet.
export function useDriverCredentialSchema(providerType: DriverId | undefined): {
  schema: CredentialFormSchema | undefined;
  supportsOAuth: boolean;
} {
  const { data: config } = useProvidersConfig();
  const entry = providerType ? config?.drivers.find((d) => d.driver === providerType) : undefined;
  return {
    schema: entry?.credential_schema,
    supportsOAuth:
      entry?.supports_oauth ?? (providerType ? providerSupportsOAuth(providerType) : false),
  };
}

export function SetApiKeyDialog({
  provider,
  open,
  onOpenChange,
}: {
  provider: Provider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { schema, supportsOAuth } = useDriverCredentialSchema(provider?.provider_type);
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const updateProvider = useUpdateProvider(provider?.id || "");

  // Same button-free probe as the add flows: a replacement key is checked before
  // it overwrites a working one, without a separate "test" step.
  const { state: credentialCheck } = useCredentialCheck({
    providerType: provider?.provider_type,
    credentials,
    baseUrl: provider?.base_url,
    enabled: open,
  });

  const oauthLabel = provider ? getProviderLabel(provider.provider_type) : "";

  // Reset to the driver's declared defaults each time the dialog opens for a
  // provider. Secrets are write-only, so existing values are never pre-filled.
  useEffect(() => {
    if (open) setCredentials(defaultCredentialValues(schema));
  }, [open, schema]);

  const handleConnectOAuth = () => {
    if (!provider) return;
    // Server redirect that sets a state cookie, so navigate the browser directly.
    window.location.href = providerOAuthAuthorizeUrl(provider.id);
  };

  const updateCredential = (key: string, value: string) =>
    setCredentials((prev) => ({ ...prev, [key]: value }));

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!provider) return;
    await updateProvider.mutateAsync({
      provider_type: provider.provider_type,
      credentials: nonEmptyCredentials(credentials),
    });
    onOpenChange(false);
    setCredentials({});
  };

  const hasInput = Object.values(credentials).some((v) => v.trim() !== "");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{provider?.api_key_set ? "Update" : "Set"} Credentials</DialogTitle>
          <DialogDescription>
            {provider?.api_key_set
              ? "Enter new credentials to replace the existing ones."
              : "Enter the credentials for this provider."}
          </DialogDescription>
        </DialogHeader>
        {supportsOAuth ? (
          <div className="space-y-3">
            <Button type="button" className="w-full" onClick={handleConnectOAuth}>
              Connect with {oauthLabel}
            </Button>
            <p className="text-sm text-muted-foreground">
              Authorize in your browser to set up a key automatically — no copy &amp; paste needed.
              Or enter credentials manually below.
            </p>
            <div className="flex items-center gap-3 py-1">
              <div className="h-px flex-1 bg-border" />
              <span className="text-xs text-muted-foreground">or</span>
              <div className="h-px flex-1 bg-border" />
            </div>
          </div>
        ) : null}
        <form onSubmit={handleSubmit} className="space-y-4">
          {schema ? (
            <div className="space-y-2">
              <CredentialFields
                schema={schema}
                values={credentials}
                onChange={updateCredential}
                idPrefix="set-credentials"
              />
              <CredentialCheckStatus state={credentialCheck} />
            </div>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={updateProvider.isPending || !hasInput}>
              {updateProvider.isPending ? "Saving..." : "Save Credentials"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
