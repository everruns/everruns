"use client";

// MCP sign-ins the viewer authorized for agent servers that act as them. They
// used to sit on the MCP page's "My connections" tab; with the catalog moved to
// Settings > Organization they live here, next to My MCP servers, so "what have
// I authorized" has one answer (knowledge/integrations/user-mcp-servers.md, UI).

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { EmptyState } from "@/components/layout";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDeleteUserConnection, useUserMcpConnections } from "@/hooks/use-user-connections";
import { registryDomainIcons } from "@/lib/registry-navigation";

const McpIcon = registryDomainIcons.mcpServers;

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function McpGrantsPanel() {
  const connections = useUserMcpConnections();
  const revokeConnection = useDeleteUserConnection();

  return (
    <section>
      <div className="mb-4">
        <h2 className="text-xl font-semibold">MCP sign-ins for agent servers</h2>
        <p className="text-sm text-muted-foreground">
          Sign-ins you authorized for organization MCP servers that agents use as you.
        </p>
      </div>
      <QueryStateWrapper
        isLoading={connections.isLoading}
        error={connections.error}
        data={connections.data ?? []}
        errorMessagePrefix="Failed to load your MCP connections"
        loadingSkeleton={<Skeleton className="h-32 w-full" />}
        emptyState={
          <EmptyState
            icon={<McpIcon />}
            title="No MCP connections"
            description="Connections you authorize for acts-as-user MCP attachments will appear here."
          />
        }
      >
        {(items) => (
          <div className="space-y-3">
            <div className="border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Server</TableHead>
                    <TableHead>Host</TableHead>
                    <TableHead>Account</TableHead>
                    <TableHead>Scopes</TableHead>
                    <TableHead>Connected</TableHead>
                    <TableHead>State</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {items.map((connection) => {
                    const unavailable = connection.server_status === "deleted";
                    return (
                      <TableRow key={connection.provider}>
                        <TableCell className="font-medium">
                          {unavailable ? "Preset unavailable" : connection.server_name}
                        </TableCell>
                        <TableCell className="font-mono text-xs">
                          {hostOf(connection.server_url)}
                        </TableCell>
                        <TableCell>{connection.provider_username || "—"}</TableCell>
                        <TableCell>{connection.scopes || "—"}</TableCell>
                        <TableCell>
                          {new Date(connection.connected_at).toLocaleDateString()}
                        </TableCell>
                        <TableCell>
                          <Badge variant={unavailable ? "secondary" : "outline"}>
                            {unavailable ? "unavailable" : "connected"}
                          </Badge>
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={revokeConnection.isPending}
                            onClick={() => revokeConnection.mutate(connection.provider)}
                          >
                            Revoke
                          </Button>
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
            {connections.hasNextPage && (
              <div className="flex justify-center">
                <Button
                  variant="outline"
                  disabled={connections.isFetchingNextPage}
                  onClick={() => connections.fetchNextPage()}
                >
                  {connections.isFetchingNextPage ? "Loading…" : "Load more connections"}
                </Button>
              </div>
            )}
          </div>
        )}
      </QueryStateWrapper>
    </section>
  );
}
