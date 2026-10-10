"use client";

// Settings > Organization > MCP catalog: the admin registry of MCP presets.
// It moved here from the main navigation (knowledge/integrations/user-mcp-servers.md,
// step 8). People who cannot manage it do not see the Settings entry; personal MCP
// servers and grants live in Settings > My agent experience.

import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import { Button, LinkButton } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { SearchInput } from "@/components/ui/search-input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useCheckMcpServerConnection,
  useMcpServerCatalog,
  useDestroyMcpServer,
} from "@/hooks/use-mcp-servers";
import { McpConnectionCheckLine } from "@/components/mcp/mcp-connection-check";
import { usePolicies } from "@/hooks/use-policies";
import { usePageTitle } from "@/hooks";
import { Plus, Trash2, Key, Pencil, Wrench } from "lucide-react";
import type { McpServer, McpServerCatalogEntry, McpProtocolMode } from "@/lib/api/types";
import { getEntityNameClassName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { SectionTabs, EmptyState, IconTile } from "@/components/layout";
import { registryDomainIcons } from "@/lib/registry-navigation";
import { pluralize } from "@/lib/formatting";
import { EntityIdentity } from "@/components/ui/entity-identity";
import {
  AddMcpServerDialog,
  ArchiveConfirmDialog,
  EditMcpServerDialog,
  ManageHeadersDialog,
  SetApiKeyDialog,
} from "@/components/mcp/mcp-catalog-dialogs";
import { McpToolLabelsDialog } from "@/components/mcp/mcp-tool-labels-dialog";

const McpIcon = registryDomainIcons.mcpServers;

/** Human-readable label for the protocol-era policy. Undefined means `auto`. */
function protocolModeLabel(mode?: McpProtocolMode): string {
  switch (mode) {
    case "2025-03-26":
    case "2025-06-18":
    case "2026-07-28":
      return mode;
    default:
      return "Auto";
  }
}

function McpServerRow({
  server,
  canManage,
  canDestroy,
  onEdit,
  onDelete,
  onArchive,
  onSetApiKey,
  onManageHeaders,
  onTools,
  onCheckConnection,
  checkingConnection,
}: {
  server: McpServerCatalogEntry;
  canManage: boolean;
  canDestroy: boolean;
  onEdit: (server: McpServer) => void;
  onDelete: (server: McpServer) => void;
  onArchive: (server: McpServer) => void;
  onSetApiKey: (server: McpServer) => void;
  onManageHeaders: (server: McpServer) => void;
  onTools: (server: McpServer) => void;
  onCheckConnection: (server: McpServer) => void;
  checkingConnection: boolean;
}) {
  const isArchived = server.status === "archived";
  const isDeleted = server.status === "deleted";
  const canEdit = canManage && !isArchived && !isDeleted;
  const host = (() => {
    try {
      return new URL(server.url).host;
    } catch {
      return server.url;
    }
  })();
  const openEdit = () => {
    if (canEdit) onEdit(server);
  };

  return (
    <TableRow
      className={canEdit ? "cursor-pointer" : undefined}
      tabIndex={canEdit ? 0 : undefined}
      onClick={openEdit}
      onKeyDown={(event) => {
        if (canEdit && (event.key === "Enter" || event.key === " ")) {
          event.preventDefault();
          openEdit();
        }
      }}
    >
      <TableCell className="py-2.5">
        <div className="flex items-center gap-3">
          <IconTile size="md" icon={<McpIcon />} />
          <div className="min-w-0">
            <div className="font-medium">
              <EntityIdentity
                value={server.id}
                labelClassName={getEntityNameClassName(server.status)}
              >
                {server.name}
              </EntityIdentity>
            </div>
            <div className="max-w-[32ch] truncate text-xs text-foreground/75">
              {server.description || "Catalog preset"}
            </div>
          </div>
        </div>
      </TableCell>
      <TableCell className="font-mono text-xs">{host}</TableCell>
      <TableCell>
        <div className="flex flex-col gap-1">
          <Badge variant="secondary" className="w-fit font-mono">
            {server.transport_type.toUpperCase()}
          </Badge>
          <span className="text-xs text-muted-foreground">
            {protocolModeLabel(server.protocol_mode)}
          </span>
        </div>
      </TableCell>
      <TableCell>
        <div className="flex flex-col items-start gap-1">
          <Badge variant="outline">{server.auth_mode.replace("_", " ")}</Badge>
          {server.auth_mode === "oauth" && (
            <McpConnectionCheckLine
              check={server.connection_check}
              canCheck={canEdit}
              checking={checkingConnection}
              onCheck={() => onCheckConnection(server)}
            />
          )}
        </div>
      </TableCell>
      <TableCell>
        <span
          className="whitespace-nowrap"
          title="Counts active agents with an explicit attachment to this catalog preset. Archived agents are excluded."
        >
          {server.used_by_agents} {pluralize(server.used_by_agents, "agent")}
        </span>
      </TableCell>
      <TableCell>
        <EntityStatus status={server.status} />
      </TableCell>
      <TableCell>
        <div className="flex items-center justify-end gap-2">
          {canManage && server.auth_mode === "api_key" && (
            <Button
              variant="outline"
              size="sm"
              onClick={(event) => {
                event.stopPropagation();
                onSetApiKey(server);
              }}
            >
              <Key className="h-4 w-4 mr-1" />
              {server.api_key_set ? "Update Key" : "Set Key"}
            </Button>
          )}
          {!isDeleted && (
            <Button
              variant="outline"
              size="sm"
              onClick={(event) => {
                event.stopPropagation();
                onTools(server);
              }}
            >
              <Wrench className="h-4 w-4 mr-1" />
              Tools
            </Button>
          )}
          {canEdit && (
            <Button
              variant="outline"
              size="sm"
              onClick={(event) => {
                event.stopPropagation();
                onEdit(server);
              }}
            >
              <Pencil className="h-4 w-4 mr-1" />
              Edit
            </Button>
          )}
          {canEdit && (
            <Button
              variant="outline"
              size="sm"
              onClick={(event) => {
                event.stopPropagation();
                onManageHeaders(server);
              }}
            >
              Headers
            </Button>
          )}
          {canEdit && (
            <Button
              variant="outline"
              size="sm"
              onClick={(event) => {
                event.stopPropagation();
                onArchive(server);
              }}
            >
              Archive
            </Button>
          )}
          {isArchived && canDestroy && (
            <Button
              variant="destructive"
              size="sm"
              onClick={(event) => {
                event.stopPropagation();
                onDelete(server);
              }}
            >
              <Trash2 className="h-4 w-4 mr-1" />
              Delete
            </Button>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}

function McpServerRowSkeleton() {
  return (
    <TableRow>
      <TableCell className="py-2.5">
        <div className="flex items-center gap-3">
          <Skeleton className="h-8 w-8" />
          <div className="space-y-2">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-3 w-40" />
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Skeleton className="h-5 w-14" />
      </TableCell>
      <TableCell>
        <Skeleton className="h-5 w-14" />
      </TableCell>
      <TableCell>
        <Skeleton className="h-5 w-16" />
      </TableCell>
      <TableCell>
        <Skeleton className="ml-auto h-8 w-24" />
      </TableCell>
    </TableRow>
  );
}

type StatusTab = "all" | "active" | "archived";

export default function McpCatalogPage() {
  usePageTitle("MCP catalog", "Settings");
  const policies = usePolicies("mcp-servers");
  const policyLoading = policies.isLoading === true;
  const canViewCatalog = policies.can("mcp_server.view");
  const canManage = policies.can("mcp_server.manage");
  const canDestroy = policies.can("mcp_server.dangerous");
  const catalog = useMcpServerCatalog(!policyLoading && canViewCatalog);
  const destroyServer = useDestroyMcpServer();
  const checkConnection = useCheckMcpServerConnection();

  const [search, setSearch] = useState("");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [addServerOpen, setAddServerOpen] = useState(false);
  const [editServer, setEditServer] = useState<McpServer | null>(null);
  const [apiKeyServer, setApiKeyServer] = useState<McpServer | null>(null);
  const [headersServer, setHeadersServer] = useState<McpServer | null>(null);
  const [toolsServer, setToolsServer] = useState<McpServer | null>(null);
  const [pendingDeleteServer, setPendingDeleteServer] = useState<McpServer | null>(null);
  const [pendingArchiveServer, setPendingArchiveServer] = useState<McpServer | null>(null);

  const handleDeleteServer = async () => {
    if (!pendingDeleteServer) return;
    await destroyServer.mutateAsync(pendingDeleteServer.id);
    setPendingDeleteServer(null);
  };

  const counts = useMemo(() => {
    const list = catalog.data ?? [];
    const archived = list.filter((server) => isArchivedStatus(server.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [catalog.data]);

  const filteredServers = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (catalog.data ?? []).filter((server) => {
      if (statusTab === "active" && isArchivedStatus(server.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(server.status)) return false;
      if (!query) return true;
      return [server.name, server.description, server.url]
        .filter(Boolean)
        .join(" ")
        .toLowerCase()
        .includes(query);
    });
  }, [catalog.data, search, statusTab]);

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  const loadingTable = (
    <Table>
      <TableBody>
        {[...Array(3)].map((_, index) => (
          <McpServerRowSkeleton key={index} />
        ))}
      </TableBody>
    </Table>
  );

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <h2 className="text-xl font-semibold">MCP catalog</h2>
          <p className="text-sm text-muted-foreground">
            MCP server presets your organization registers once, with their transport and
            authentication, so agents and people can add them by name.
          </p>
          <p className="text-sm text-muted-foreground">
            A preset does nothing until an agent or a person adds it.
          </p>
        </div>
        {canViewCatalog && canManage && (
          <Button variant="accent" onClick={() => setAddServerOpen(true)}>
            <Plus className="size-4" />
            Add Server
          </Button>
        )}
      </div>

      {policyLoading ? (
        loadingTable
      ) : !canViewCatalog ? (
        <EmptyState
          icon={<McpIcon />}
          title="You cannot view the MCP catalog"
          description="Ask an organization admin for access. MCP servers you add for yourself live in Settings, My agent experience."
          action={
            <LinkButton href="/settings/agent-experience" variant="outline">
              Open My agent experience
            </LinkButton>
          }
        />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3">
            <SearchInput
              placeholder="Search catalog…"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              containerClassName="w-64"
            />
            <div className="flex-1" />
            <SectionTabs
              value={statusTab}
              onValueChange={(value) => setStatusTab(value as StatusTab)}
              items={statusItems.map((item) => ({
                ...item,
                count: counts[item.value],
              }))}
            />
          </div>
          <QueryStateWrapper
            isLoading={catalog.isLoading}
            error={catalog.error}
            data={filteredServers}
            errorMessagePrefix="Failed to load MCP catalog"
            loadingSkeleton={loadingTable}
            emptyState={
              <EmptyState
                icon={<McpIcon />}
                title={search ? "No catalog presets match your search" : "No MCP presets"}
                description={
                  search
                    ? undefined
                    : "Add an MCP preset so agents can attach a shared transport and authentication policy."
                }
                action={
                  !search && canManage ? (
                    <Button variant="accent" onClick={() => setAddServerOpen(true)}>
                      <Plus className="size-4" />
                      Add Server
                    </Button>
                  ) : undefined
                }
              />
            }
          >
            {(items) => (
              <div className="space-y-3">
                <div className="border">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Name</TableHead>
                        <TableHead>Host</TableHead>
                        <TableHead>Transport / era</TableHead>
                        <TableHead>Auth</TableHead>
                        <TableHead>
                          <span title="Active agents only. Archived agents are excluded.">
                            Used by
                          </span>
                        </TableHead>
                        <TableHead>Status</TableHead>
                        <TableHead className="text-right">Actions</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {items.map((server) => (
                        <McpServerRow
                          key={server.id}
                          server={server}
                          canManage={canManage}
                          canDestroy={canDestroy}
                          onEdit={setEditServer}
                          onDelete={setPendingDeleteServer}
                          onArchive={setPendingArchiveServer}
                          onSetApiKey={setApiKeyServer}
                          onManageHeaders={setHeadersServer}
                          onTools={setToolsServer}
                          onCheckConnection={(target) => checkConnection.mutate(target.id)}
                          checkingConnection={
                            checkConnection.isPending && checkConnection.variables === server.id
                          }
                        />
                      ))}
                    </TableBody>
                  </Table>
                </div>
                {catalog.hasNextPage && (
                  <div className="flex justify-center">
                    <Button
                      variant="outline"
                      disabled={catalog.isFetchingNextPage}
                      onClick={() => catalog.fetchNextPage()}
                    >
                      {catalog.isFetchingNextPage ? "Loading…" : "Load more presets"}
                    </Button>
                  </div>
                )}
              </div>
            )}
          </QueryStateWrapper>
        </>
      )}

      <AddMcpServerDialog open={addServerOpen} onOpenChange={setAddServerOpen} />
      <EditMcpServerDialog
        server={editServer}
        open={editServer !== null}
        onOpenChange={(open) => !open && setEditServer(null)}
      />
      <ManageHeadersDialog
        server={headersServer}
        open={headersServer !== null}
        onOpenChange={(open) => !open && setHeadersServer(null)}
      />
      {toolsServer && (
        <McpToolLabelsDialog
          server={toolsServer}
          canEdit={canManage && !isArchivedStatus(toolsServer.status)}
          open
          onOpenChange={(open) => !open && setToolsServer(null)}
        />
      )}
      <ArchiveConfirmDialog
        server={pendingArchiveServer}
        open={pendingArchiveServer !== null}
        onOpenChange={(open) => !open && setPendingArchiveServer(null)}
      />
      <SetApiKeyDialog
        server={apiKeyServer}
        open={apiKeyServer !== null}
        onOpenChange={(open) => !open && setApiKeyServer(null)}
      />
      <Dialog
        open={pendingDeleteServer !== null}
        onOpenChange={(open) => !open && setPendingDeleteServer(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete MCP Server</DialogTitle>
            <DialogDescription>
              Permanently delete the archived MCP server{" "}
              <span className="font-medium">{pendingDeleteServer?.name}</span>? Existing references
              will render as deleted tombstones.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingDeleteServer(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDeleteServer}
              disabled={destroyServer.isPending}
            >
              {destroyServer.isPending ? "Deleting..." : "Delete MCP Server"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
