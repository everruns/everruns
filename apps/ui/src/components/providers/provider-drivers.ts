"use client";

// Driver catalog helpers shared by the provider forms. Credential entry
// itself lives in provider-credentials.tsx.

import { useProvidersConfig } from "@/hooks/use-providers";
import { providerSupportsOAuth } from "@/lib/api/providers";
import type { DriverId, CredentialFormSchema } from "@/lib/api/types";

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
  const entry = providerType ? config?.drivers?.find((d) => d.driver === providerType) : undefined;
  return {
    schema: entry?.credential_schema,
    supportsOAuth:
      entry?.supports_oauth ?? (providerType ? providerSupportsOAuth(providerType) : false),
  };
}
