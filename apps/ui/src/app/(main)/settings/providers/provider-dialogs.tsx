"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useCreateProvider, useProvidersConfig, useUpdateProvider } from "@/hooks/use-providers";
import {
  ProviderIcon,
  getProviderLabel,
  getProviderDescription,
} from "@/components/providers/provider-icon";
import { providerSupportsOAuth, providerOAuthAuthorizeUrl } from "@/lib/api/providers";
import type {
  Provider,
  DriverId,
  CreateProviderRequest,
  CredentialFormSchema,
} from "@/lib/api/types";
import {
  CredentialFields,
  defaultCredentialValues,
  nonEmptyCredentials,
} from "./credential-fields";

// Ordered list of provider types for the picker. Labels and descriptions come
// from the centralized `getProviderLabel` / `getProviderDescription` mappings so
// the dropdown and the selected-value trigger never diverge.
const PROVIDER_TYPES: DriverId[] = [
  "openai",
  "openrouter",
  "azure_openai",
  "openai_completions",
  "anthropic",
  "gemini",
  "bedrock",
  "mai",
  "fireworks",
];

// Drivers whose endpoint (`base_url`) is mandatory.
const BASE_URL_REQUIRED: DriverId[] = ["azure_openai", "mai"];

function getBaseUrlPlaceholder(providerType: DriverId): string {
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
    default:
      return "https://api.example.com";
  }
}

// Resolve a driver's declared credential schema (and whether it supports OAuth)
// from the providers config. Falls back to the client OAuth shim when the
// config has not loaded yet.
function useDriverCredentialSchema(providerType: DriverId | undefined): {
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

export function AddProviderDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [providerType, setProviderType] = useState<DriverId>("openai");
  const [baseUrl, setBaseUrl] = useState("");
  const [credentials, setCredentials] = useState<Record<string, string>>({});

  const { schema, supportsOAuth } = useDriverCredentialSchema(providerType);
  const createProvider = useCreateProvider();

  // Reset credential inputs to the selected driver's declared defaults whenever
  // the driver (or its schema) changes.
  useEffect(() => {
    setCredentials(defaultCredentialValues(schema));
  }, [schema, providerType]);

  const baseUrlRequired = BASE_URL_REQUIRED.includes(providerType);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const submitted = nonEmptyCredentials(credentials);
    const data: CreateProviderRequest = {
      name,
      provider_type: providerType,
      base_url: baseUrl || undefined,
      credentials: Object.keys(submitted).length > 0 ? submitted : undefined,
    };
    await createProvider.mutateAsync(data);
    onOpenChange(false);
    setName("");
    setProviderType("openai");
    setBaseUrl("");
    setCredentials({});
  };

  const updateCredential = (key: string, value: string) =>
    setCredentials((prev) => ({ ...prev, [key]: value }));

  // Create the provider, then hand the browser straight to the driver's OAuth
  // authorize endpoint. The flow needs a provider id to bind its state cookie
  // to, so creation has to happen first; doing both in one click is why OAuth is
  // actionable here instead of a "create first, then connect" hint. The dialog
  // unmounts on navigation, so local form state needs no reset.
  const handleCreateAndConnect = async () => {
    const submitted = nonEmptyCredentials(credentials);
    const data: CreateProviderRequest = {
      name,
      provider_type: providerType,
      base_url: baseUrl || undefined,
      credentials: Object.keys(submitted).length > 0 ? submitted : undefined,
    };
    const provider = await createProvider.mutateAsync(data);
    window.location.href = providerOAuthAuthorizeUrl(provider.id);
  };

  const submitDisabled = createProvider.isPending || !name || (baseUrlRequired && !baseUrl.trim());

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Provider</DialogTitle>
          <DialogDescription>Configure a new provider for your agents to use.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="name">Name</Label>
            <Input
              id="name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="My OpenAI Provider"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="provider-type">Provider Type</Label>
            <Select value={providerType} onValueChange={(v) => setProviderType(v as DriverId)}>
              <SelectTrigger className="w-full">
                <div className="flex items-center gap-2">
                  <ProviderIcon providerType={providerType} size="sm" showBackground={false} />
                  <span>{getProviderLabel(providerType)}</span>
                </div>
              </SelectTrigger>
              <SelectContent>
                {PROVIDER_TYPES.map((type) => (
                  <SelectItem key={type} value={type}>
                    <div className="flex items-center gap-2">
                      <ProviderIcon providerType={type} size="sm" showBackground={false} />
                      <div className="flex flex-col">
                        <span>{getProviderLabel(type)}</span>
                        <span className="text-xs text-muted-foreground">
                          {getProviderDescription(type)}
                        </span>
                      </div>
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-sm text-muted-foreground">{getProviderDescription(providerType)}</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="base-url">Base URL{baseUrlRequired ? "" : " (optional)"}</Label>
            <Input
              id="base-url"
              value={baseUrl}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setBaseUrl(e.target.value)}
              placeholder={getBaseUrlPlaceholder(providerType)}
              required={baseUrlRequired}
            />
            {providerType === "azure_openai" ? (
              <p className="text-sm text-muted-foreground">
                Use your Azure OpenAI `.../openai/v1` endpoint on `openai.azure.com` or
                `services.ai.azure.com`.
              </p>
            ) : null}
          </div>
          {schema ? (
            <CredentialFields
              schema={schema}
              values={credentials}
              onChange={updateCredential}
              idPrefix="add-provider"
              allowEmptySubmit={supportsOAuth}
            />
          ) : null}
          {supportsOAuth ? (
            <div className="space-y-2">
              <Button
                type="button"
                className="w-full"
                onClick={handleCreateAndConnect}
                disabled={submitDisabled}
              >
                Connect with {getProviderLabel(providerType)}
              </Button>
              <p className="text-sm text-muted-foreground">
                Authorize in your browser to set up a key automatically — no copy &amp; paste
                needed. Or paste a key above and use “Create Provider”.
              </p>
            </div>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={submitDisabled}>
              {createProvider.isPending ? "Creating..." : "Create Provider"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
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
            <CredentialFields
              schema={schema}
              values={credentials}
              onChange={updateCredential}
              idPrefix="set-credentials"
            />
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
