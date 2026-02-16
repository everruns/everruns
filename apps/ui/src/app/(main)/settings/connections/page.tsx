"use client";

import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { useUserConnections, useDeleteUserConnection } from "@/hooks/use-user-connections";
import { getBackendUrl } from "@/lib/api/client";
import { ExternalLink, Github, LinkIcon, Trash2, Check } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { UserConnection } from "@/lib/api/types";

/** Provider metadata for display */
const providers: Record<string, { name: string; icon: LucideIcon; description: string }> = {
  github: {
    name: "GitHub",
    icon: Github,
    description: "Access private repositories for agent sessions",
  },
};

function ProviderIcon({ provider, className }: { provider: string; className?: string }) {
  const meta = providers[provider];
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
  const meta = providers[connection.provider];
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
    // Navigate to OAuth authorize endpoint through the API proxy
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
        Connect
      </Button>
    </div>
  );
}

export default function ConnectionsPage() {
  const { data: connections = [], isLoading, error } = useUserConnections();
  const deleteConnection = useDeleteUserConnection();
  const searchParams = useSearchParams();
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Show success toast when redirected back from OAuth
  useEffect(() => {
    const connected = searchParams.get("connected");
    if (connected) {
      const meta = providers[connected];
      setSuccessMessage(`${meta?.name ?? connected} connected successfully`);
      // Clear URL param without reload
      window.history.replaceState({}, "", "/settings/connections");
      const timer = setTimeout(() => setSuccessMessage(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [searchParams]);

  const handleDisconnect = async (provider: string) => {
    const meta = providers[provider];
    const name = meta?.name ?? provider;
    if (confirm(`Disconnect ${name}? New sessions will no longer have access to your ${name} account.`)) {
      await deleteConnection.mutateAsync(provider);
    }
  };

  // Determine which providers are not yet connected
  const connectedProviders = new Set(connections.map((c) => c.provider));
  const availableProviders = Object.entries(providers).filter(
    ([key]) => !connectedProviders.has(key),
  );

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
            <p className="text-muted-foreground">
              No connection providers are configured.
            </p>
          </Card>
        ) : (
          <div className="space-y-2">
            {connections.map((conn) => (
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

      {/* Available providers */}
      {availableProviders.length > 0 && (
        <section>
          <div className="mb-4">
            <h2 className="text-lg font-semibold">Available</h2>
            <p className="text-sm text-muted-foreground">
              Connect additional accounts to enable repository access in sessions.
            </p>
          </div>
          <div className="space-y-2">
            {availableProviders.map(([key, meta]) => (
              <AvailableProviderRow key={key} provider={key} meta={meta} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
