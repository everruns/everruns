"use client";

// External AI clients (Claude, ChatGPT, Cursor, ...) the viewer approved to act
// as them on /mcp, with Revoke. The other direction, servers Everruns agents
// call as the viewer, is My MCP servers above it
// (knowledge/integrations/mcp-connected-clients.md, phase 1).

import { useState } from "react";
import { Plug } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { useConnectedClients, useRevokeConnectedClient } from "@/hooks/use-connected-clients";
import { formatRelativeTime, formatShortDate } from "@/lib/formatting";
import type { ConnectedClient } from "@/lib/api/types";

function ClientInitial({ name }: { name: string }) {
  const initial = name.trim().charAt(0).toUpperCase() || "?";
  return (
    <span
      aria-hidden="true"
      className="flex size-7 shrink-0 items-center justify-center border bg-muted text-xs font-semibold text-muted-foreground"
    >
      {initial}
    </span>
  );
}

export function ConnectedClientsPanel() {
  const clients = useConnectedClients();
  const revoke = useRevokeConnectedClient();
  const [pending, setPending] = useState<ConnectedClient | null>(null);

  const confirmRevoke = async () => {
    if (!pending) return;
    try {
      await revoke.mutateAsync(pending.id);
      setPending(null);
    } catch {
      // Shown in the dialog through revoke.error.
    }
  };

  const askToRevoke = (client: ConnectedClient) => {
    revoke.reset();
    setPending(client);
  };

  return (
    <section>
      <div className="mb-4">
        <h2 className="text-xl font-semibold">Connected AI clients</h2>
        <p className="text-sm text-muted-foreground">
          Apps like Claude, ChatGPT and Cursor that you approved to use Everruns MCP as you, in
          every organization you belong to.
        </p>
      </div>
      <QueryStateWrapper
        isLoading={clients.isLoading}
        error={clients.error}
        data={clients.data ?? []}
        errorMessagePrefix="Failed to load your connected AI clients"
        loadingSkeleton={<Skeleton className="h-32 w-full" />}
        emptyState={
          <EmptyState
            icon={<Plug />}
            title="No connected AI clients"
            description="When you approve an AI client to use Everruns MCP, it appears here and you can disconnect it."
          />
        }
      >
        {(items) => (
          <div className="border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Client</TableHead>
                  <TableHead>Redirects to</TableHead>
                  <TableHead>Approved</TableHead>
                  <TableHead>Last used</TableHead>
                  <TableHead>Access</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((client) => (
                  <TableRow key={client.id}>
                    <TableCell>
                      <div className="flex items-center gap-2 font-medium">
                        <ClientInitial name={client.client_name} />
                        <span>{client.client_name}</span>
                      </div>
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {client.redirect_hosts.length > 0 ? client.redirect_hosts.join(", ") : "—"}
                    </TableCell>
                    <TableCell>{formatShortDate(client.created_at)}</TableCell>
                    <TableCell>
                      {client.last_used_at ? formatRelativeTime(client.last_used_at) : "Never"}
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        <Badge variant="outline">
                          {client.access === "read_only" ? "Read only" : "Read and run"}
                        </Badge>
                        {client.all_organizations && (
                          <Badge variant="outline">All organizations</Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="text-right">
                      <Button variant="outline" size="sm" onClick={() => askToRevoke(client)}>
                        Revoke
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </QueryStateWrapper>
      <Dialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) setPending(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disconnect {pending?.client_name}?</DialogTitle>
            <DialogDescription>
              <span className="font-medium">{pending?.client_name}</span> will lose access to
              Everruns within a minute. To use it again, you will need to approve it again.
            </DialogDescription>
          </DialogHeader>
          {revoke.error && (
            <p className="text-sm text-destructive">
              The client could not be disconnected: {revoke.error.message}
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setPending(null)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={confirmRevoke} disabled={revoke.isPending}>
              {revoke.isPending ? "Revoking…" : "Revoke"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
