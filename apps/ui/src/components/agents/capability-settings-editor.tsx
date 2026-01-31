"use client";

import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Check, AlertCircle, Loader2 } from "lucide-react";
import type { CapabilityId } from "@/lib/api/types";
import {
  setCapabilitySecret,
  checkCapabilitySecretExists,
} from "@/lib/api/capability-secrets";

// Known capability IDs that have configurable settings
const DOCKER_CONTAINER_ID = "docker_container";
const WEB_SEARCH_ID = "web_search";

// Type definitions for capability-specific configs
export interface DockerContainerConfig {
  image?: string;
  working_dir?: string;
}

export interface WebSearchConfig {
  provider?: "brave" | "tavily" | "exa";
}

export type CapabilityConfig =
  | DockerContainerConfig
  | WebSearchConfig
  | Record<string, unknown>;

interface CapabilitySettingsEditorProps {
  /** The capability ID */
  capabilityId: CapabilityId;
  /** Current config value */
  config: Record<string, unknown>;
  /** Callback when config changes */
  onChange: (config: Record<string, unknown>) => void;
  /** Whether editing is disabled */
  disabled?: boolean;
  /** Agent ID (required for secret storage) */
  agentId?: string;
}

/**
 * Editor for capability-specific configuration.
 * Renders appropriate form fields based on the capability type.
 */
export function CapabilitySettingsEditor({
  capabilityId,
  config,
  onChange,
  disabled,
  agentId,
}: CapabilitySettingsEditorProps) {
  // Render appropriate editor based on capability type
  switch (capabilityId) {
    case DOCKER_CONTAINER_ID:
      return (
        <DockerContainerEditor
          config={config as DockerContainerConfig}
          onChange={onChange}
          disabled={disabled}
        />
      );
    case WEB_SEARCH_ID:
      return (
        <WebSearchEditor
          config={config as WebSearchConfig}
          onChange={onChange}
          disabled={disabled}
          agentId={agentId}
        />
      );
    default:
      // No settings editor for this capability
      return null;
  }
}

/**
 * Check if a capability has configurable settings
 */
export function hasCapabilitySettings(capabilityId: CapabilityId): boolean {
  return capabilityId === DOCKER_CONTAINER_ID || capabilityId === WEB_SEARCH_ID;
}

// ============================================================================
// Docker Container Config Editor
// ============================================================================

const DEFAULT_DOCKER_IMAGE = "mcr.microsoft.com/devcontainers/python:3.11";
const DEFAULT_WORKING_DIR = "/workspace";

interface DockerContainerEditorProps {
  config: DockerContainerConfig;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

function DockerContainerEditor({ config, onChange, disabled }: DockerContainerEditorProps) {
  const handleImageChange = (value: string) => {
    // Only set the value if it's different from default, to keep config clean
    const newConfig = { ...config };
    if (value && value !== DEFAULT_DOCKER_IMAGE) {
      newConfig.image = value;
    } else {
      delete newConfig.image;
    }
    onChange(newConfig);
  };

  const handleWorkingDirChange = (value: string) => {
    const newConfig = { ...config };
    if (value && value !== DEFAULT_WORKING_DIR) {
      newConfig.working_dir = value;
    } else {
      delete newConfig.working_dir;
    }
    onChange(newConfig);
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1.5">
        <Label htmlFor="docker-image" className="text-xs font-normal text-muted-foreground">
          Docker Image
        </Label>
        <Input
          id="docker-image"
          placeholder={DEFAULT_DOCKER_IMAGE}
          value={config.image || ""}
          onChange={(e) => handleImageChange(e.target.value)}
          disabled={disabled}
          className="h-8 text-sm"
        />
        <p className="text-xs text-muted-foreground">Custom base image for the container</p>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="docker-working-dir" className="text-xs font-normal text-muted-foreground">
          Working Directory
        </Label>
        <Input
          id="docker-working-dir"
          placeholder={DEFAULT_WORKING_DIR}
          value={config.working_dir || ""}
          onChange={(e) => handleWorkingDirChange(e.target.value)}
          disabled={disabled}
          className="h-8 text-sm"
        />
        <p className="text-xs text-muted-foreground">
          Default working directory inside the container
        </p>
      </div>
    </div>
  );
}

// ============================================================================
// Web Search Config Editor
// ============================================================================

const WEB_SEARCH_PROVIDERS = [
  { value: "brave", label: "Brave Search", description: "Privacy-focused web search" },
  { value: "tavily", label: "Tavily", description: "AI-optimized search for LLMs" },
  { value: "exa", label: "Exa", description: "Neural search with semantic understanding" },
] as const;

type WebSearchProvider = (typeof WEB_SEARCH_PROVIDERS)[number]["value"];

interface WebSearchEditorProps {
  config: WebSearchConfig;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
  agentId?: string;
}

function WebSearchEditor({ config, onChange, disabled, agentId }: WebSearchEditorProps) {
  const [apiKey, setApiKey] = useState("");
  const [hasApiKey, setHasApiKey] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<"idle" | "success" | "error">("idle");
  const [errorMessage, setErrorMessage] = useState("");

  // Check if API key exists when agent ID is available
  useEffect(() => {
    if (agentId) {
      setIsLoading(true);
      checkCapabilitySecretExists(agentId, WEB_SEARCH_ID, "api_key")
        .then((exists) => {
          setHasApiKey(exists);
        })
        .catch((err) => {
          console.error("Failed to check API key:", err);
        })
        .finally(() => {
          setIsLoading(false);
        });
    }
  }, [agentId]);

  const handleProviderChange = (value: string) => {
    const newConfig: WebSearchConfig = { ...config, provider: value as WebSearchProvider };
    onChange(newConfig as Record<string, unknown>);
  };

  const handleSaveApiKey = async () => {
    if (!agentId || !apiKey.trim()) return;

    setIsSaving(true);
    setSaveStatus("idle");
    setErrorMessage("");

    try {
      await setCapabilitySecret(agentId, WEB_SEARCH_ID, "api_key", apiKey.trim());
      setHasApiKey(true);
      setApiKey("");
      setSaveStatus("success");
      setTimeout(() => setSaveStatus("idle"), 3000);
    } catch (err) {
      console.error("Failed to save API key:", err);
      setSaveStatus("error");
      setErrorMessage(err instanceof Error ? err.message : "Failed to save API key");
    } finally {
      setIsSaving(false);
    }
  };

  const selectedProvider = WEB_SEARCH_PROVIDERS.find((p) => p.value === config.provider);

  return (
    <div className="space-y-4 pt-2">
      {/* Provider Selection */}
      <div className="space-y-1.5">
        <Label htmlFor="web-search-provider" className="text-xs font-normal text-muted-foreground">
          Search Provider
        </Label>
        <Select
          value={config.provider || "brave"}
          onValueChange={handleProviderChange}
          disabled={disabled}
        >
          <SelectTrigger id="web-search-provider" className="h-8 text-sm">
            <SelectValue placeholder="Select a provider" />
          </SelectTrigger>
          <SelectContent>
            {WEB_SEARCH_PROVIDERS.map((provider) => (
              <SelectItem key={provider.value} value={provider.value}>
                <div className="flex flex-col">
                  <span>{provider.label}</span>
                </div>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {selectedProvider && (
          <p className="text-xs text-muted-foreground">{selectedProvider.description}</p>
        )}
      </div>

      {/* API Key Section */}
      <div className="space-y-1.5">
        <Label htmlFor="web-search-api-key" className="text-xs font-normal text-muted-foreground">
          API Key
        </Label>

        {/* Status indicator */}
        {isLoading ? (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-3 w-3 animate-spin" />
            <span>Checking API key status...</span>
          </div>
        ) : hasApiKey ? (
          <div className="flex items-center gap-2 text-xs text-green-600">
            <Check className="h-3 w-3" />
            <span>API key is configured</span>
          </div>
        ) : (
          <div className="flex items-center gap-2 text-xs text-amber-600">
            <AlertCircle className="h-3 w-3" />
            <span>API key not configured</span>
          </div>
        )}

        {/* API Key Input */}
        {agentId && (
          <div className="flex gap-2 mt-2">
            <Input
              id="web-search-api-key"
              type="password"
              placeholder={hasApiKey ? "Enter new API key to update" : "Enter API key"}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              disabled={disabled || isSaving}
              className="h-8 text-sm flex-1"
            />
            <Button
              size="sm"
              variant="secondary"
              onClick={handleSaveApiKey}
              disabled={disabled || isSaving || !apiKey.trim()}
              className="h-8"
            >
              {isSaving ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                "Save"
              )}
            </Button>
          </div>
        )}

        {/* Save Status */}
        {saveStatus === "success" && (
          <p className="text-xs text-green-600">API key saved successfully</p>
        )}
        {saveStatus === "error" && (
          <p className="text-xs text-red-600">{errorMessage || "Failed to save API key"}</p>
        )}

        {/* Help text */}
        <p className="text-xs text-muted-foreground mt-1">
          {config.provider === "brave" && (
            <>Get your API key from <a href="https://brave.com/search/api/" target="_blank" rel="noopener noreferrer" className="underline">Brave Search API</a></>
          )}
          {config.provider === "tavily" && (
            <>Get your API key from <a href="https://tavily.com/" target="_blank" rel="noopener noreferrer" className="underline">Tavily</a></>
          )}
          {config.provider === "exa" && (
            <>Get your API key from <a href="https://exa.ai/" target="_blank" rel="noopener noreferrer" className="underline">Exa</a></>
          )}
          {!config.provider && (
            <>Select a provider to see where to get the API key</>
          )}
        </p>

        {/* Note for new agents */}
        {!agentId && (
          <p className="text-xs text-muted-foreground italic">
            Save the agent first, then configure the API key
          </p>
        )}
      </div>
    </div>
  );
}
