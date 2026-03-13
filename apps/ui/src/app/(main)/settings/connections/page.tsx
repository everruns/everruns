"use client";

import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useUserConnections,
  useDeleteUserConnection,
  useConnectionProviders,
  useVerifyConnection,
} from "@/hooks/use-user-connections";
import { getBackendUrl } from "@/lib/api/client";
import {
  ExternalLink,
  LinkIcon,
  Trash2,
  Check,
  CheckCircle,
  ShieldCheck,
  XCircle,
  Loader2,
} from "lucide-react";
import type { UserConnection, ConnectionProvider as ConnectionProviderType } from "@/lib/api/types";
import { ProviderIcon } from "@/components/connections/provider-icon";
import { ApiKeyDialog } from "@/components/connections/api-key-dialog";

function ConnectionRow({
  connection,
  provider,
  onDisconnect,
  isDisconnecting,
}: {
  connection: UserConnection;
  provider?: ConnectionProviderType;
  onDisconnect: (provider: string) => void;
  isDisconnecting: boolean;
}) {
  const displayName = provider?.display_name ?? connection.provider;
  const icon = provider?.icon ?? "link";
  const isApiKey = connection.connection_type === "api_key";
  const verify = useVerifyConnection();
  const [verifyStatus, setVerifyStatus] = useState<"idle" | "valid" | "invalid">("idle");
  const [verifyError, setVerifyError] = useState<string | null>(null);

  const handleVerify = async () => {
    setVerifyStatus("idle");
    setVerifyError(null);
    try {
      const result = await verify.mutateAsync(connection.provider);
      if (result.valid) {
        setVerifyStatus("valid");
      } else {
        setVerifyStatus("invalid");
        setVerifyError(result.error ?? "Verification failed");
      }
    } catch {
      setVerifyStatus("invalid");
      setVerifyError("Failed to verify connection");
    }
  };

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-3">
        <ProviderIcon iconName={icon} className="h-5 w-5" />
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
          {verifyStatus === "valid" && (
            <div className="flex items-center gap-1 text-green-600 dark:text-green-400 text-xs mt-1">
              <CheckCircle className="h-3 w-3" />
              API key is valid
            </div>
          )}
          {verifyStatus === "invalid" && (
            <div className="flex items-center gap-1 text-destructive text-xs mt-1">
              <XCircle className="h-3 w-3" />
              {verifyError}
            </div>
          )}
        </div>
      </div>
      <div className="flex items-center gap-2">
        {isApiKey && (
          <Button variant="outline" size="sm" onClick={handleVerify} disabled={verify.isPending}>
            {verify.isPending ? (
              <Loader2 className="h-4 w-4 mr-1 animate-spin" />
            ) : (
              <ShieldCheck className="h-4 w-4 mr-1" />
            )}
            {verify.isPending ? "Verifying..." : "Verify"}
          </Button>
        )}
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
    </div>
  );
}

function AvailableProviderRow({
  provider,
  onConnectApiKey,
}: {
  provider: ConnectionProviderType;
  onConnectApiKey: (provider: ConnectionProviderType) => void;
}) {
  const handleConnect = () => {
    if (provider.connection_type === "oauth") {
      window.location.href = `${getBackendUrl()}/v1/user/connections/${provider.provider_id}/authorize`;
    } else {
      onConnectApiKey(provider);
    }
  };

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-3">
        <ProviderIcon iconName={provider.icon} className="h-5 w-5" />
        <div>
          <div className="font-medium">{provider.display_name}</div>
          <div className="text-sm text-muted-foreground">{provider.description}</div>
        </div>
      </div>
      <Button variant="outline" size="sm" onClick={handleConnect}>
        {provider.connection_type === "oauth" ? (
          <ExternalLink className="h-4 w-4 mr-1" />
        ) : (
          <LinkIcon className="h-4 w-4 mr-1" />
        )}
        Connect
      </Button>
    </div>
  );
}

export default function ConnectionsPage() {
  const { data: connections = [], isLoading, error } = useUserConnections();
  const { data: providers = [] } = useConnectionProviders();
  const deleteConnection = useDeleteUserConnection();
  const searchParams = useSearchParams();
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [apiKeyProvider, setApiKeyProvider] = useState<ConnectionProviderType | null>(null);

  // Show success toast when redirected back from OAuth
  useEffect(() => {
    const connected = searchParams.get("connected");
    if (connected) {
      const provider = providers.find((p) => p.provider_id === connected);
      setSuccessMessage(`${provider?.display_name ?? connected} connected successfully`);
      window.history.replaceState({}, "", "/settings/connections");
      const timer = setTimeout(() => setSuccessMessage(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [searchParams, providers]);

  const handleDisconnect = async (provider: string) => {
    const meta = providers.find((p) => p.provider_id === provider);
    const name = meta?.display_name ?? provider;
    if (
      confirm(
        `Disconnect ${name}? New sessions will no longer have access to your ${name} account.`,
      )
    ) {
      await deleteConnection.mutateAsync(provider);
    }
  };

  // Providers not yet connected
  const connectedProviders = new Set(connections.map((c) => c.provider));
  const availableProviders = providers.filter((p) => !connectedProviders.has(p.provider_id));

  // Build provider lookup for connected rows
  const providerMap = new Map(providers.map((p) => [p.provider_id, p]));

  return (
    <div className="space-y-8">
      {/* Success banner */}
      {successMessage && (
        <div className="flex items-center gap-2 bg-green-500/10 text-green-700 dark:text-green-400 p-3 rounded-lg text-sm">
          <Check className="h-4 w-4" />
          {successMessage}
        </div>
      )}

      {/* Connected accounts */}
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
        ) : connections.length === 0 && availableProviders.length === 0 ? (
          <Card className="p-8 text-center">
            <LinkIcon className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No connections available</h3>
            <p className="text-muted-foreground">No connection providers are configured.</p>
          </Card>
        ) : (
          <div className="space-y-2">
            {connections.map((conn) => (
              <ConnectionRow
                key={conn.provider}
                connection={conn}
                provider={providerMap.get(conn.provider)}
                onDisconnect={handleDisconnect}
                isDisconnecting={deleteConnection.isPending}
              />
            ))}
          </div>
        )}
      </section>

      {/* Available providers */}
      {availableProviders.length > 0 && (
        <section>
          <div className="mb-4">
            <h2 className="text-lg font-semibold">Available</h2>
            <p className="text-sm text-muted-foreground">
              Connect additional accounts to enable access in sessions.
            </p>
          </div>
          <div className="space-y-2">
            {availableProviders.map((provider) => (
              <AvailableProviderRow
                key={provider.provider_id}
                provider={provider}
                onConnectApiKey={setApiKeyProvider}
              />
            ))}
          </div>
        </section>
      )}

      {/* API Key entry dialog */}
      <ApiKeyDialog
        provider={apiKeyProvider}
        open={apiKeyProvider !== null}
        onOpenChange={(open) => {
          if (!open) setApiKeyProvider(null);
        }}
      />
    </div>
  );
}
