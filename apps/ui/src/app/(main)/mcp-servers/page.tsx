"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useMcpServers,
  useCreateMcpServer,
  useUpdateMcpServer,
  useDestroyMcpServer,
} from "@/hooks/use-mcp-servers";
import { usePolicies } from "@/hooks/use-policies";
import { Plus, Plug, Trash2, Key, Globe } from "lucide-react";
import type { McpServer, CreateMcpServerRequest, McpServerAuthMode } from "@/lib/api/types";
import {
  apiKeySecretSchema,
  getFieldErrors,
  mcpServerFormSchema,
  type FieldErrors,
} from "@/lib/form-validation";
import { getEntityNameClassName, getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import { ArchiveFilter } from "@/components/archive-filter";

function McpServerCard({
  server,
  canDestroy,
  onDelete,
  onArchive,
  onSetApiKey,
}: {
  server: McpServer;
  canDestroy: boolean;
  onDelete: (server: McpServer) => void;
  onArchive: (server: McpServer) => void;
  onSetApiKey: (server: McpServer) => void;
}) {
  const isArchived = server.status === "archived";
  const isDeleted = server.status === "deleted";

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center bg-primary/10">
            <Plug className="h-5 w-5 text-primary" />
          </div>
          <div>
            <CardTitle className={`text-lg ${getEntityNameClassName(server.status)}`}>
              {server.name}
            </CardTitle>
            <CardDescription className="text-sm">
              {server.description || "No description"}
            </CardDescription>
          </div>
        </div>
        <Badge variant={getEntityStatusBadgeVariant(server.status)}>{server.status}</Badge>
      </CardHeader>
      <CardContent>
        <div className="space-y-2 text-sm">
          <div className="flex items-center gap-2">
            <Globe className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground truncate">{server.url}</span>
          </div>
          <div className="flex items-center gap-2">
            <Key className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground">
              Auth:{" "}
              {server.auth_mode === "oauth"
                ? "OAuth"
                : server.auth_mode === "api_key"
                  ? server.api_key_set
                    ? "API key configured"
                    : "API key missing"
                  : "None"}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="text-xs">
              {server.transport_type.toUpperCase()}
            </Badge>
          </div>
        </div>
        <div className="flex items-center justify-end gap-2 mt-4">
          {server.auth_mode === "api_key" && (
            <Button variant="outline" size="sm" onClick={() => onSetApiKey(server)}>
              <Key className="h-4 w-4 mr-1" />
              {server.api_key_set ? "Update Key" : "Set Key"}
            </Button>
          )}
          {!isArchived && !isDeleted && (
            <Button variant="outline" size="sm" onClick={() => onArchive(server)}>
              Archive
            </Button>
          )}
          {isArchived && canDestroy && (
            <Button variant="destructive" size="sm" onClick={() => onDelete(server)}>
              <Trash2 className="h-4 w-4 mr-1" />
              Delete
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function AddMcpServerDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [url, setUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [authMode, setAuthMode] = useState<McpServerAuthMode>("none");
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  const createServer = useCreateMcpServer();

  useEffect(() => {
    if (!open) return;
    setName("");
    setDescription("");
    setUrl("");
    setApiKey("");
    setAuthMode("none");
    setFieldErrors({});
  }, [open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const parsed = mcpServerFormSchema.safeParse({
      name,
      description,
      url,
      auth_mode: authMode,
      api_key: apiKey,
    });
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    const data: CreateMcpServerRequest = {
      name: parsed.data.name,
      description: parsed.data.description,
      url: parsed.data.url,
      transport_type: "http",
      auth_mode: parsed.data.auth_mode,
      api_key: parsed.data.auth_mode === "api_key" ? parsed.data.api_key : undefined,
    };
    await createServer.mutateAsync(data);
    onOpenChange(false);
    setName("");
    setDescription("");
    setUrl("");
    setApiKey("");
    setAuthMode("none");
    setFieldErrors({});
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add MCP Server</DialogTitle>
          <DialogDescription>
            Configure a new MCP server connection. Currently only HTTP (Streamable HTTP) servers are
            supported.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="name">Name</Label>
            <Input
              id="name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                setName(e.target.value);
                setFieldErrors((prev) => ({ ...prev, name: undefined }));
              }}
              aria-invalid={!!fieldErrors.name}
              placeholder="atlassian-mcp-server"
              required
            />
            {fieldErrors.name && <p className="text-xs text-destructive">{fieldErrors.name}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="description">Description (optional)</Label>
            <Textarea
              id="description"
              value={description}
              onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => {
                setDescription(e.target.value);
                setFieldErrors((prev) => ({ ...prev, description: undefined }));
              }}
              aria-invalid={!!fieldErrors.description}
              placeholder="Atlassian MCP Server for Jira and Confluence"
              rows={2}
            />
            {fieldErrors.description && (
              <p className="text-xs text-destructive">{fieldErrors.description}</p>
            )}
          </div>
          <div className="space-y-2">
            <Label htmlFor="url">URL</Label>
            <Input
              id="url"
              value={url}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                setUrl(e.target.value);
                setFieldErrors((prev) => ({ ...prev, url: undefined }));
              }}
              aria-invalid={!!fieldErrors.url}
              placeholder="https://mcp.atlassian.com/v1/mcp"
              required
            />
            {fieldErrors.url && <p className="text-xs text-destructive">{fieldErrors.url}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="auth-mode">Authentication</Label>
            <Select
              value={authMode}
              onValueChange={(value) => {
                setAuthMode(value as McpServerAuthMode);
                setFieldErrors((prev) => ({ ...prev, auth_mode: undefined, api_key: undefined }));
              }}
            >
              <SelectTrigger id="auth-mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                <SelectItem value="api_key">API Key</SelectItem>
                <SelectItem value="oauth">OAuth per user</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {authMode === "api_key" && (
            <div className="space-y-2">
              <Label htmlFor="api-key">API Key</Label>
              <Input
                id="api-key"
                type="password"
                value={apiKey}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                  setApiKey(e.target.value);
                  setFieldErrors((prev) => ({ ...prev, api_key: undefined }));
                }}
                aria-invalid={!!fieldErrors.api_key}
                placeholder="your-api-key"
                required
              />
              {fieldErrors.api_key && (
                <p className="text-xs text-destructive">{fieldErrors.api_key}</p>
              )}
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                createServer.isPending || !name || !url || (authMode === "api_key" && !apiKey)
              }
            >
              {createServer.isPending ? "Creating..." : "Create Server"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function SetApiKeyDialog({
  server,
  open,
  onOpenChange,
}: {
  server: McpServer | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});
  const updateServer = useUpdateMcpServer(server?.id || "");

  useEffect(() => {
    if (!open) return;
    setApiKey("");
    setFieldErrors({});
  }, [open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!server) return;

    const parsed = apiKeySecretSchema.safeParse({ api_key: apiKey });
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    await updateServer.mutateAsync({ api_key: parsed.data.api_key });
    onOpenChange(false);
    setApiKey("");
    setFieldErrors({});
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{server?.api_key_set ? "Update" : "Set"} API Key</DialogTitle>
          <DialogDescription>
            {server?.api_key_set
              ? "Enter a new API key to replace the existing one."
              : "Enter the API key for this MCP server."}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="new-api-key">API Key</Label>
            <Input
              id="new-api-key"
              type="password"
              value={apiKey}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                setApiKey(e.target.value);
                setFieldErrors((prev) => ({ ...prev, api_key: undefined }));
              }}
              aria-invalid={!!fieldErrors.api_key}
              placeholder="your-api-key"
              required
            />
            {fieldErrors.api_key && (
              <p className="text-xs text-destructive">{fieldErrors.api_key}</p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={updateServer.isPending || !apiKey}>
              {updateServer.isPending ? "Saving..." : "Save API Key"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ArchiveConfirmDialog({
  server,
  open,
  onOpenChange,
}: {
  server: McpServer | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const updateServer = useUpdateMcpServer(server?.id || "");

  const handleArchive = async () => {
    if (!server) return;
    await updateServer.mutateAsync({ status: "archived" });
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Archive MCP Server</DialogTitle>
          <DialogDescription>
            Are you sure you want to archive the MCP server{" "}
            <span className="font-medium">{server?.name}</span>? Archived servers will no longer be
            available to agents.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleArchive} disabled={updateServer.isPending}>
            {updateServer.isPending ? "Archiving..." : "Archive"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function McpServerCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-24" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full mb-4" />
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}

export default function McpServersPage() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: servers, isLoading, error } = useMcpServers({ includeArchived: showArchived });
  const destroyServer = useDestroyMcpServer();
  const { can: canPolicies } = usePolicies("mcp-servers");
  const canDestroy = canPolicies("mcp_server.dangerous");

  const [addServerOpen, setAddServerOpen] = useState(false);
  const [apiKeyServer, setApiKeyServer] = useState<McpServer | null>(null);
  const [pendingDeleteServer, setPendingDeleteServer] = useState<McpServer | null>(null);
  const [pendingArchiveServer, setPendingArchiveServer] = useState<McpServer | null>(null);

  const handleDeleteServer = async () => {
    if (!pendingDeleteServer) return;
    await destroyServer.mutateAsync(pendingDeleteServer.id);
    setPendingDeleteServer(null);
  };

  return (
    <div className="container mx-auto p-6 space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold">MCP Servers</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Configure Model Context Protocol (MCP) servers to extend agent capabilities with
            external tools and resources.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Button onClick={() => setAddServerOpen(true)}>
            <Plus className="mr-2 h-4 w-4" />
            Add Server
          </Button>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={servers}
        errorMessagePrefix="Failed to load MCP servers"
        loadingSkeleton={
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {[...Array(3)].map((_, i) => (
              <McpServerCardSkeleton key={i} />
            ))}
          </div>
        }
        emptyState={
          <Card className="p-8 text-center">
            <Plug className="mx-auto mb-4 h-12 w-12 text-muted-foreground" />
            <h2 className="mb-2 text-lg font-medium">No MCP servers configured</h2>
            <p className="mb-4 text-muted-foreground">
              Add an MCP server to extend your agents with external tools and resources.
            </p>
            <Button onClick={() => setAddServerOpen(true)}>
              <Plus className="mr-2 h-4 w-4" />
              Add Server
            </Button>
          </Card>
        }
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((server) => (
              <McpServerCard
                key={server.id}
                server={server}
                canDestroy={canDestroy}
                onDelete={setPendingDeleteServer}
                onArchive={setPendingArchiveServer}
                onSetApiKey={setApiKeyServer}
              />
            ))}
          </div>
        )}
      </QueryStateWrapper>

      {/* Dialogs */}
      <AddMcpServerDialog open={addServerOpen} onOpenChange={setAddServerOpen} />
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
