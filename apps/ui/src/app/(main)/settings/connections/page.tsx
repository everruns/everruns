"use client";

import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useUserConnections,
  useDeleteUserConnection,
  usePutApiKeyConnection,
} from "@/hooks/use-user-connections";
import { useCapabilities } from "@/hooks/use-capabilities";
import { getBackendUrl } from "@/lib/api/client";
import { ExternalLink, Github, LinkIcon, Trash2, Check, AlertCircle, Key } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { UserConnection, ConnectionProviderInfo } from "@/lib/api/types";

/** Static provider metadata (always shown regardless of integrations) */
const staticProviders: Record<string, { name: string; icon: LucideIcon; description: string }> = {
  github: {
    name: "GitHub",
    icon: Github,
    description: "Install the GitHub App to grant access to selected repositories",
  },
};

function ProviderIcon({ provider, className }: { provider: string; className?: string }) {
  const meta = staticProviders[provider];
  if (meta) {
    const Icon = meta.icon;
    return <Icon className={className} />;
  }
  return <LinkIcon className={className} />;
}

function ConnectionRow({
  connection,
  onDisconnect,
  isDisconnecting,
}: {
  connection: UserConnection;
  onDisconnect: (provider: string) => void;
  isDisconnecting: boolean;
}) {
  const meta = staticProviders[connection.provider];
  const displayName = meta?.name ?? connection.provider;

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-3">
        <ProviderIcon provider={connection.provider} className="h-5 w-5" />
        <div>
          <div className="font-medium flex items-center gap-2">
            {displayName}
            {connection.provider_username && (
              <Badge variant="outline" className="text-xs font-normal">
                {connection.provider_username}
              </Badge>
            )}
          </div>
          <div className="text-sm text-muted-foreground">
            Connected {new Date(connection.connected_at).toLocaleDateString()}
            {connection.scopes && <span className="ml-2">({connection.scopes})</span>}
          </div>
        </div>
      </div>
      <Button
        variant="ghost"
        size="sm"
        className="text-destructive"
        onClick={() => onDisconnect(connection.provider)}
        disabled={isDisconnecting}
      >
        <Trash2 className="h-4 w-4 mr-1" />
        Disconnect
      </Button>
    </div>
  );
}

function AvailableProviderRow({
  provider,
  meta,
}: {
  provider: string;
  meta: { name: string; icon: LucideIcon; description: string };
}) {
  const handleConnect = () => {
    // Navigate to GitHub App installation flow through the API proxy
    window.location.href = `${getBackendUrl()}/v1/user/connections/${provider}/authorize`;
  };

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-3">
        <ProviderIcon provider={provider} className="h-5 w-5" />
        <div>
          <div className="font-medium">{meta.name}</div>
          <div className="text-sm text-muted-foreground">{meta.description}</div>
        </div>
      </div>
      <Button variant="outline" size="sm" onClick={handleConnect}>
        <ExternalLink className="h-4 w-4 mr-1" />
        Install
      </Button>
    </div>
  );
}

/** Row for API-key-based connections from capabilities */
function ApiKeyProviderRow({
  provider,
  isConnected,
  onSave,
  isSaving,
  onDisconnect,
  isDisconnecting,
  connectedAt,
}: {
  provider: ConnectionProviderInfo;
  isConnected: boolean;
  onSave: (providerId: string, apiKey: string) => void;
  isSaving: boolean;
  onDisconnect: (providerId: string) => void;
  isDisconnecting: boolean;
  connectedAt?: string;
}) {
  const [apiKey, setApiKey] = useState("");
  const [isEditing, setIsEditing] = useState(false);

  if (isConnected && !isEditing) {
    return (
      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex items-center gap-3">
          <Key className="h-5 w-5" />
          <div>
            <div className="font-medium">{provider.name}</div>
            <div className="text-sm text-muted-foreground">
              API key configured
              {connectedAt && ` \u00b7 ${new Date(connectedAt).toLocaleDateString()}`}
            </div>
          </div>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => setIsEditing(true)}>
            Update
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive"
            onClick={() => onDisconnect(provider.id)}
            disabled={isDisconnecting}
          >
            <Trash2 className="h-4 w-4 mr-1" />
            Remove
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-3 flex-1 min-w-0">
        <Key className="h-5 w-5 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="font-medium">{provider.name}</div>
          <div className="text-sm text-muted-foreground">{provider.description}</div>
          <div className="flex items-center gap-2 mt-2">
            <Input
              type="password"
              placeholder="Enter API key"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              className="max-w-xs h-8 text-sm"
            />
            <Button
              size="sm"
              disabled={!apiKey.trim() || isSaving}
              onClick={() => {
                onSave(provider.id, apiKey);
                setApiKey("");
                setIsEditing(false);
              }}
            >
              Save
            </Button>
            {isEditing && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setIsEditing(false);
                  setApiKey("");
                }}
              >
                Cancel
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default function ConnectionsPage() {
  const { data: connections = [], isLoading, error } = useUserConnections();
  const { data: capabilities = [] } = useCapabilities();
  const deleteConnection = useDeleteUserConnection();
  const putApiKey = usePutApiKeyConnection();
  const searchParams = useSearchParams();
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Show toast when redirected back from connection flow
  useEffect(() => {
    const connected = searchParams.get("connected");
    const errorParam = searchParams.get("error");
    if (connected) {
      const meta = staticProviders[connected];
      setSuccessMessage(`${meta?.name ?? connected} connected successfully`);
      window.history.replaceState({}, "", "/settings/connections");
      const timer = setTimeout(() => setSuccessMessage(null), 4000);
      return () => clearTimeout(timer);
    }
    if (errorParam === "github_not_configured") {
      setErrorMessage(
        "GitHub App is not configured. Set GITHUB_APP_ID and GITHUB_APP_PRIVATE_KEY to enable GitHub connections.",
      );
      window.history.replaceState({}, "", "/settings/connections");
      const timer = setTimeout(() => setErrorMessage(null), 8000);
      return () => clearTimeout(timer);
    }
  }, [searchParams]);

  const handleDisconnect = async (provider: string) => {
    const meta = staticProviders[provider];
    const name = meta?.name ?? provider;
    if (
      confirm(
        `Disconnect ${name}? New sessions will no longer have access to your ${name} account.`,
      )
    ) {
      await deleteConnection.mutateAsync(provider);
    }
  };

  const handleApiKeySave = async (providerId: string, apiKey: string) => {
    await putApiKey.mutateAsync({ provider: providerId, apiKey });
    setSuccessMessage(`API key saved for ${providerId}`);
    const timer = setTimeout(() => setSuccessMessage(null), 4000);
    return () => clearTimeout(timer);
  };

  const handleApiKeyDisconnect = async (providerId: string) => {
    if (confirm(`Remove API key for ${providerId}? Sessions will no longer have access.`)) {
      await deleteConnection.mutateAsync(providerId);
    }
  };

  // Determine which static providers (GitHub) are not yet connected
  const connectedProviders = new Set(connections.map((c) => c.provider));
  const availableStaticProviders = Object.entries(staticProviders).filter(
    ([key]) => !connectedProviders.has(key),
  );

  // Extract API key connection providers from loaded capabilities
  const apiKeyProviders: ConnectionProviderInfo[] = capabilities
    .filter((cap) => cap.connection_provider)
    .map((cap) => cap.connection_provider!);

  const hasAnyContent =
    connections.length > 0 || availableStaticProviders.length > 0 || apiKeyProviders.length > 0;

  return (
    <div className="space-y-8">
      {/* Success banner */}
      {successMessage && (
        <div className="flex items-center gap-2 bg-green-500/10 text-green-700 dark:text-green-400 p-3 rounded-lg text-sm">
          <Check className="h-4 w-4" />
          {successMessage}
        </div>
      )}

      {/* Error banner */}
      {errorMessage && (
        <div className="flex items-center gap-2 bg-destructive/10 text-destructive p-3 rounded-lg text-sm">
          <AlertCircle className="h-4 w-4 flex-shrink-0" />
          {errorMessage}
        </div>
      )}

      {/* Connected accounts (OAuth-style: GitHub) */}
      <section>
        <div className="mb-4">
          <h2 className="text-xl font-semibold">Connections</h2>
          <p className="text-sm text-muted-foreground">
            Connected accounts are automatically available in agent sessions.
          </p>
        </div>

        {error && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-4">
            Failed to load connections: {error.message}
          </div>
        )}

        {isLoading ? (
          <div className="space-y-2">
            {[...Array(1)].map((_, i) => (
              <Skeleton key={i} className="h-[72px] w-full" />
            ))}
          </div>
        ) : !hasAnyContent ? (
          <Card className="p-8 text-center">
            <LinkIcon className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No connections available</h3>
            <p className="text-muted-foreground">No connection providers are configured.</p>
          </Card>
        ) : (
          <div className="space-y-2">
            {/* Show connected OAuth accounts (GitHub) */}
            {connections
              .filter((conn) => conn.provider in staticProviders)
              .map((conn) => (
                <ConnectionRow
                  key={conn.provider}
                  connection={conn}
                  onDisconnect={handleDisconnect}
                  isDisconnecting={deleteConnection.isPending}
                />
              ))}
          </div>
        )}
      </section>

      {/* Available static providers (GitHub not yet connected) */}
      {availableStaticProviders.length > 0 && (
        <section>
          <div className="mb-4">
            <h2 className="text-lg font-semibold">Available</h2>
            <p className="text-sm text-muted-foreground">
              Connect additional accounts to enable repository access in sessions.
            </p>
          </div>
          <div className="space-y-2">
            {availableStaticProviders.map(([key, meta]) => (
              <AvailableProviderRow key={key} provider={key} meta={meta} />
            ))}
          </div>
        </section>
      )}

      {/* API Key connections (from loaded integrations) */}
      {apiKeyProviders.length > 0 && (
        <section>
          <div className="mb-4">
            <h2 className="text-lg font-semibold">API Keys</h2>
            <p className="text-sm text-muted-foreground">
              API keys for integrations. Stored encrypted and available in all sessions.
            </p>
          </div>
          <div className="space-y-2">
            {apiKeyProviders.map((provider) => {
              const conn = connections.find((c) => c.provider === provider.id);
              return (
                <ApiKeyProviderRow
                  key={provider.id}
                  provider={provider}
                  isConnected={!!conn}
                  connectedAt={conn?.connected_at}
                  onSave={handleApiKeySave}
                  isSaving={putApiKey.isPending}
                  onDisconnect={handleApiKeyDisconnect}
                  isDisconnecting={deleteConnection.isPending}
                />
              );
            })}
          </div>
        </section>
      )}
    </div>
  );
}
