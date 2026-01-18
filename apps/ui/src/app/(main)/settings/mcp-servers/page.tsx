"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
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
  useDeleteMcpServer,
  useMcpServerOAuthStatus,
  useStartMcpServerOAuth,
  useRevokeMcpServerOAuth,
} from "@/hooks/use-mcp-servers";
import { Plus, Plug, Trash2, Key, Globe, Shield, LogIn, LogOut, ExternalLink } from "lucide-react";
import type { McpServer, CreateMcpServerRequest } from "@/lib/api/types";

function OAuthStatusBadge({ server }: { server: McpServer }) {
  const { data: oauthStatus, isLoading } = useMcpServerOAuthStatus(server.id);

  if (server.auth_type !== "oauth") {
    return null;
  }

  if (isLoading) {
    return <Skeleton className="h-5 w-20" />;
  }

  if (oauthStatus?.authorized) {
    return (
      <Badge variant="outline" className="bg-blue-100 text-blue-800">
        <Shield className="h-3 w-3 mr-1" />
        Connected
      </Badge>
    );
  }

  return (
    <Badge variant="outline" className="bg-orange-100 text-orange-800">
      <Shield className="h-3 w-3 mr-1" />
      Authorization Required
    </Badge>
  );
}

function OAuthActions({ server }: { server: McpServer }) {
  const { data: oauthStatus, isLoading } = useMcpServerOAuthStatus(server.id);
  const startOAuth = useStartMcpServerOAuth(server.id);
  const revokeOAuth = useRevokeMcpServerOAuth(server.id);

  if (server.auth_type !== "oauth") {
    return null;
  }

  const handleAuthorize = async () => {
    try {
      // Get the return URL (current page)
      const returnUrl = window.location.href;
      const response = await startOAuth.mutateAsync(returnUrl);
      // Redirect to OAuth provider
      window.location.href = response.authorization_url;
    } catch (error) {
      console.error("Failed to start OAuth flow:", error);
    }
  };

  const handleRevoke = async () => {
    if (confirm("Are you sure you want to revoke OAuth access for this MCP server?")) {
      await revokeOAuth.mutateAsync();
    }
  };

  if (isLoading) {
    return <Skeleton className="h-8 w-24" />;
  }

  if (oauthStatus?.authorized) {
    return (
      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={handleAuthorize}
          disabled={startOAuth.isPending}
        >
          <ExternalLink className="h-4 w-4 mr-1" />
          Re-authorize
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          onClick={handleRevoke}
          disabled={revokeOAuth.isPending}
        >
          <LogOut className="h-4 w-4" />
        </Button>
      </div>
    );
  }

  return (
    <Button
      variant="default"
      size="sm"
      onClick={handleAuthorize}
      disabled={startOAuth.isPending}
    >
      <LogIn className="h-4 w-4 mr-1" />
      {startOAuth.isPending ? "Starting..." : "Authorize"}
    </Button>
  );
}

function McpServerCard({
  server,
  onDelete,
  onSetApiKey,
}: {
  server: McpServer;
  onDelete: (id: string) => void;
  onSetApiKey: (server: McpServer) => void;
}) {
  const authType = server.auth_type || "none";

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10">
            <Plug className="h-5 w-5 text-primary" />
          </div>
          <div>
            <CardTitle className="text-lg">{server.name}</CardTitle>
            <CardDescription className="text-sm">
              {server.description || "No description"}
            </CardDescription>
          </div>
        </div>
        <div className="flex flex-col items-end gap-1">
          <Badge
            variant="outline"
            className={
              server.status === "active" ? "bg-green-100 text-green-800" : "bg-gray-100 text-gray-800"
            }
          >
            {server.status}
          </Badge>
          <OAuthStatusBadge server={server} />
        </div>
      </CardHeader>
      <CardContent>
        <div className="space-y-2 text-sm">
          <div className="flex items-center gap-2">
            <Globe className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground truncate">{server.url}</span>
          </div>
          <div className="flex items-center gap-2">
            <Shield className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground">
              Auth: {authType === "api_key" ? "API Key" : authType === "oauth" ? "OAuth" : "None"}
              {authType === "api_key" && (
                <span className="ml-1">
                  ({server.api_key_set ? "Configured" : "Not set"})
                </span>
              )}
            </span>
          </div>
          {server.oauth_scopes && server.oauth_scopes.length > 0 && (
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-muted-foreground text-xs">Scopes:</span>
              {server.oauth_scopes.map((scope) => (
                <Badge key={scope} variant="secondary" className="text-xs">
                  {scope}
                </Badge>
              ))}
            </div>
          )}
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="text-xs">
              {server.transport_type.toUpperCase()}
            </Badge>
          </div>
        </div>
        <div className="flex items-center justify-end gap-2 mt-4">
          {authType === "oauth" && <OAuthActions server={server} />}
          {authType === "api_key" && (
            <Button variant="outline" size="sm" onClick={() => onSetApiKey(server)}>
              <Key className="h-4 w-4 mr-1" />
              {server.api_key_set ? "Update Key" : "Set Key"}
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive"
            onClick={() => onDelete(server.id)}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
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
  const [authType, setAuthType] = useState<"none" | "api_key" | "oauth">("none");
  const [apiKey, setApiKey] = useState("");
  // OAuth config fields
  const [oauthClientId, setOauthClientId] = useState("");
  const [oauthClientSecret, setOauthClientSecret] = useState("");
  const [oauthScopes, setOauthScopes] = useState("");
  const [oauthResourceMetadataUrl, setOauthResourceMetadataUrl] = useState("");

  const createServer = useCreateMcpServer();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateMcpServerRequest = {
      name,
      description: description || undefined,
      url,
      transport_type: "http",
      auth_type: authType,
      api_key: authType === "api_key" && apiKey ? apiKey : undefined,
      oauth_config: authType === "oauth" ? {
        client_id: oauthClientId || undefined,
        client_secret: oauthClientSecret || undefined,
        scopes: oauthScopes ? oauthScopes.split(",").map(s => s.trim()) : undefined,
        resource_metadata_url: oauthResourceMetadataUrl || undefined,
      } : undefined,
    };
    await createServer.mutateAsync(data);
    onOpenChange(false);
    // Reset form
    setName("");
    setDescription("");
    setUrl("");
    setAuthType("none");
    setApiKey("");
    setOauthClientId("");
    setOauthClientSecret("");
    setOauthScopes("");
    setOauthResourceMetadataUrl("");
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg max-h-[90vh] overflow-y-auto">
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
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="github-mcp-server"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="description">Description (optional)</Label>
            <Textarea
              id="description"
              value={description}
              onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) =>
                setDescription(e.target.value)
              }
              placeholder="Atlassian MCP Server for Jira and Confluence"
              rows={2}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="url">URL</Label>
            <Input
              id="url"
              value={url}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setUrl(e.target.value)}
              placeholder="https://api.githubcopilot.com/mcp/"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="auth-type">Authentication Type</Label>
            <select
              id="auth-type"
              value={authType}
              onChange={(e) => setAuthType(e.target.value as "none" | "api_key" | "oauth")}
              className="w-full h-10 px-3 py-2 text-sm border border-input bg-background rounded-md focus:outline-none focus:ring-2 focus:ring-ring"
            >
              <option value="none">None</option>
              <option value="api_key">API Key</option>
              <option value="oauth">OAuth 2.1</option>
            </select>
          </div>

          {authType === "api_key" && (
            <div className="space-y-2">
              <Label htmlFor="api-key">API Key</Label>
              <Input
                id="api-key"
                type="password"
                value={apiKey}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => setApiKey(e.target.value)}
                placeholder="your-api-key"
              />
            </div>
          )}

          {authType === "oauth" && (
            <div className="space-y-4 pt-2 border-t">
              <p className="text-sm text-muted-foreground">
                OAuth servers use RFC 9728 Protected Resource Metadata for auto-discovery.
                You can optionally provide manual configuration.
              </p>
              <div className="space-y-2">
                <Label htmlFor="oauth-resource-url">
                  Resource Metadata URL (for auto-discovery)
                </Label>
                <Input
                  id="oauth-resource-url"
                  value={oauthResourceMetadataUrl}
                  onChange={(e: React.ChangeEvent<HTMLInputElement>) => setOauthResourceMetadataUrl(e.target.value)}
                  placeholder="/.well-known/oauth-protected-resource"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="oauth-client-id">Client ID (optional)</Label>
                <Input
                  id="oauth-client-id"
                  value={oauthClientId}
                  onChange={(e: React.ChangeEvent<HTMLInputElement>) => setOauthClientId(e.target.value)}
                  placeholder="your-client-id"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="oauth-client-secret">Client Secret (optional)</Label>
                <Input
                  id="oauth-client-secret"
                  type="password"
                  value={oauthClientSecret}
                  onChange={(e: React.ChangeEvent<HTMLInputElement>) => setOauthClientSecret(e.target.value)}
                  placeholder="your-client-secret"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="oauth-scopes">Scopes (comma-separated, optional)</Label>
                <Input
                  id="oauth-scopes"
                  value={oauthScopes}
                  onChange={(e: React.ChangeEvent<HTMLInputElement>) => setOauthScopes(e.target.value)}
                  placeholder="read,write,admin"
                />
              </div>
            </div>
          )}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createServer.isPending || !name || !url}>
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
  const updateServer = useUpdateMcpServer(server?.id || "");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!server) return;
    await updateServer.mutateAsync({ api_key: apiKey });
    onOpenChange(false);
    setApiKey("");
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
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setApiKey(e.target.value)}
              placeholder="your-api-key"
              required
            />
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

function McpServerCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9 rounded-lg" />
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
  const { data: servers = [], isLoading, error } = useMcpServers();
  const deleteServer = useDeleteMcpServer();

  const [addServerOpen, setAddServerOpen] = useState(false);
  const [apiKeyServer, setApiKeyServer] = useState<McpServer | null>(null);

  const handleDeleteServer = async (id: string) => {
    if (confirm("Are you sure you want to delete this MCP server?")) {
      await deleteServer.mutateAsync(id);
    }
  };

  return (
    <div className="space-y-8">
      {/* MCP Servers Section */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">MCP Servers</h2>
            <p className="text-sm text-muted-foreground">
              Configure Model Context Protocol (MCP) servers to extend agent capabilities with
              external tools and resources.
            </p>
          </div>
          <Button onClick={() => setAddServerOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Add Server
          </Button>
        </div>

        {error && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-4">
            Failed to load MCP servers: {error.message}
          </div>
        )}

        {isLoading ? (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {[...Array(3)].map((_, i) => (
              <McpServerCardSkeleton key={i} />
            ))}
          </div>
        ) : servers.length === 0 ? (
          <Card className="p-8 text-center">
            <Plug className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No MCP servers configured</h3>
            <p className="text-muted-foreground mb-4">
              Add an MCP server to extend your agents with external tools and resources.
            </p>
            <Button onClick={() => setAddServerOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              Add Server
            </Button>
          </Card>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {servers.map((server) => (
              <McpServerCard
                key={server.id}
                server={server}
                onDelete={handleDeleteServer}
                onSetApiKey={setApiKeyServer}
              />
            ))}
          </div>
        )}
      </section>

      {/* Dialogs */}
      <AddMcpServerDialog open={addServerOpen} onOpenChange={setAddServerOpen} />
      <SetApiKeyDialog
        server={apiKeyServer}
        open={apiKeyServer !== null}
        onOpenChange={(open) => !open && setApiKeyServer(null)}
      />
    </div>
  );
}
