// Capability Secrets API functions
// Manages encrypted secrets for capability configurations (e.g., API keys for web search)

import { getApiBaseUrl } from "./client";

// ============================================================================
// Types
// ============================================================================

export interface SetCapabilitySecretRequest {
  value: string;
}

export interface SetCapabilitySecretResponse {
  success: boolean;
}

export interface ListCapabilitySecretsResponse {
  secrets: string[];
}

export interface SecretExistsResponse {
  exists: boolean;
}

// ============================================================================
// API Functions
// ============================================================================

/**
 * Set a capability secret (encrypted, write-only)
 */
export async function setCapabilitySecret(
  agentId: string,
  capabilityId: string,
  secretName: string,
  value: string
): Promise<SetCapabilitySecretResponse> {
  const res = await fetch(
    `${getApiBaseUrl()}/v1/agents/${agentId}/capabilities/${capabilityId}/secrets/${secretName}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      credentials: "include",
      body: JSON.stringify({ value } as SetCapabilitySecretRequest),
    }
  );

  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: "Unknown error" }));
    throw new Error(error.error || "Failed to set capability secret");
  }

  return res.json();
}

/**
 * Delete a capability secret
 */
export async function deleteCapabilitySecret(
  agentId: string,
  capabilityId: string,
  secretName: string
): Promise<void> {
  const res = await fetch(
    `${getApiBaseUrl()}/v1/agents/${agentId}/capabilities/${capabilityId}/secrets/${secretName}`,
    {
      method: "DELETE",
      credentials: "include",
    }
  );

  if (!res.ok && res.status !== 404) {
    const error = await res.json().catch(() => ({ error: "Unknown error" }));
    throw new Error(error.error || "Failed to delete capability secret");
  }
}

/**
 * List capability secret names (not values)
 */
export async function listCapabilitySecrets(
  agentId: string,
  capabilityId: string
): Promise<string[]> {
  const res = await fetch(
    `${getApiBaseUrl()}/v1/agents/${agentId}/capabilities/${capabilityId}/secrets`,
    {
      method: "GET",
      credentials: "include",
    }
  );

  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: "Unknown error" }));
    throw new Error(error.error || "Failed to list capability secrets");
  }

  const data: ListCapabilitySecretsResponse = await res.json();
  return data.secrets;
}

/**
 * Check if a capability secret exists
 */
export async function checkCapabilitySecretExists(
  agentId: string,
  capabilityId: string,
  secretName: string
): Promise<boolean> {
  const res = await fetch(
    `${getApiBaseUrl()}/v1/agents/${agentId}/capabilities/${capabilityId}/secrets/${secretName}/exists`,
    {
      method: "GET",
      credentials: "include",
    }
  );

  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: "Unknown error" }));
    throw new Error(error.error || "Failed to check capability secret");
  }

  const data: SecretExistsResponse = await res.json();
  return data.exists;
}
