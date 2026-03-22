"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  useIdentityConnections,
  useDeleteIdentityConnection,
  useVerifyIdentityConnection,
} from "@/hooks/use-identity-connections";
import { useConnectionProviders } from "@/hooks/use-user-connections";
import {
  LinkIcon,
  Trash2,
  CheckCircle,
  ShieldCheck,
  XCircle,
  Loader2,
} from "lucide-react";
import type { UserConnection, ConnectionProvider } from "@/lib/api/types";
import { ProviderIcon } from "@/components/connections/provider-icon";
import { IdentityApiKeyDialog } from "./identity-api-key-dialog";

function ConnectionRow({
  identityId,
  connection,
  provider,
  onDisconnect,
  isDisconnecting,
}: {
  identityId: string;
  connection: UserConnection;
  provider?: ConnectionProvider;
  onDisconnect: (provider: string) => void;
  isDisconnecting: boolean;
}) {
  const displayName = provider?.display_name ?? connection.provider;
  const icon = provider?.icon ?? "link";
  const isApiKey = connection.connection_type === "api_key";
  const verify = useVerifyIdentityConnection(identityId);
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
  provider: ConnectionProvider;
  onConnectApiKey: (provider: ConnectionProvider) => void;
}) {
  // Only API-key providers are supported for identity connections
  if (provider.connection_type !== "api_key") return null;

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-3">
        <ProviderIcon iconName={provider.icon} className="h-5 w-5" />
        <div>
          <div className="font-medium">{provider.display_name}</div>
          <div className="text-sm text-muted-foreground">{provider.description}</div>
        </div>
      </div>
      <Button variant="outline" size="sm" onClick={() => onConnectApiKey(provider)}>
        <LinkIcon className="h-4 w-4 mr-1" />
        Connect
      </Button>
    </div>
  );
}

export function IdentityConnections({ identityId }: { identityId: string }) {
  const { data: connections = [], isLoading, error } = useIdentityConnections(identityId);
  const { data: providers = [] } = useConnectionProviders();
  const deleteConnection = useDeleteIdentityConnection(identityId);
  const [apiKeyProvider, setApiKeyProvider] = useState<ConnectionProvider | null>(null);

  const handleDisconnect = async (provider: string) => {
    const meta = providers.find((p) => p.provider_id === provider);
    const name = meta?.display_name ?? provider;
    if (confirm(`Disconnect ${name} from this identity?`)) {
      await deleteConnection.mutateAsync(provider);
    }
  };

  // Only show API-key providers as available (OAuth not yet supported for identities)
  const connectedProviders = new Set(connections.map((c) => c.provider));
  const availableProviders = providers.filter(
    (p) => p.connection_type === "api_key" && !connectedProviders.has(p.provider_id),
  );

  const providerMap = new Map(providers.map((p) => [p.provider_id, p]));

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-lg font-semibold">Connections</h3>
        <p className="text-sm text-muted-foreground">
          Connections assigned to this identity are used in sessions running as this identity.
        </p>
      </div>

      {error && (
        <div className="bg-destructive/10 text-destructive p-4 rounded-lg">
          Failed to load connections: {error.message}
        </div>
      )}

      {isLoading ? (
        <Skeleton className="h-[72px] w-full" />
      ) : connections.length === 0 && availableProviders.length === 0 ? (
        <div className="p-6 text-center border rounded-lg">
          <LinkIcon className="h-8 w-8 mx-auto text-muted-foreground mb-2" />
          <p className="text-muted-foreground text-sm">No connection providers available.</p>
        </div>
      ) : (
        <>
          {connections.length > 0 && (
            <div className="space-y-2">
              {connections.map((conn) => (
                <ConnectionRow
                  key={conn.provider}
                  identityId={identityId}
                  connection={conn}
                  provider={providerMap.get(conn.provider)}
                  onDisconnect={handleDisconnect}
                  isDisconnecting={deleteConnection.isPending}
                />
              ))}
            </div>
          )}

          {availableProviders.length > 0 && (
            <div className="space-y-2">
              {connections.length > 0 && (
                <h4 className="text-sm font-medium text-muted-foreground pt-2">Available</h4>
              )}
              {availableProviders.map((provider) => (
                <AvailableProviderRow
                  key={provider.provider_id}
                  provider={provider}
                  onConnectApiKey={setApiKeyProvider}
                />
              ))}
            </div>
          )}
        </>
      )}

      <IdentityApiKeyDialog
        identityId={identityId}
        provider={apiKeyProvider}
        open={apiKeyProvider !== null}
        onOpenChange={(open) => {
          if (!open) setApiKeyProvider(null);
        }}
      />
    </div>
  );
}
