"use client";

/**
 * My MCP servers: MCP servers a person adds for themselves, from the
 * organization catalog or by URL. Agents that use the person's MCP servers
 * get these tools while that person is chatting.
 * See knowledge/integrations/user-mcp-servers.md.
 */

import { useEffect, useState } from "react";
import { ExternalLink, Plus, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { SectionTabs } from "@/components/layout/page-layout";
import { useMcpServerCatalog } from "@/hooks/use-mcp-servers";
import { usePolicies } from "@/hooks/use-policies";
import {
  useAddUserMcpServer,
  useRemoveUserMcpServer,
  useUpdateUserMcpServer,
  useUserMcpServers,
} from "@/hooks/use-user-mcp-servers";
import { getBackendUrl } from "@/lib/api/client";
import type { UserMcpServer } from "@/lib/api/types";
import { registryDomainIcons } from "@/lib/registry-navigation";

const McpIcon = registryDomainIcons.mcpServers;

function host(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function connectUrl(identityId: string, provider: string) {
  return `${getBackendUrl()}/v1/virtual-users/${encodeURIComponent(identityId)}/connections/${encodeURIComponent(provider)}/authorize?return_to=${encodeURIComponent(window.location.pathname)}`;
}

function SignInBadge({ server }: { server: UserMcpServer }) {
  switch (server.connection.status) {
    case "connected":
      return <Badge variant="success">Signed in</Badge>;
    case "not_connected":
      return <Badge variant="outline">Needs sign-in</Badge>;
    default:
      return null;
  }
}

function ServerRow({ identityId, server }: { identityId: string; server: UserMcpServer }) {
  const update = useUpdateUserMcpServer(identityId);
  const remove = useRemoveUserMcpServer(identityId);
  const provider = server.connection.provider;
  const canConnect = server.connection.status === "not_connected" && provider;

  return (
    <div className="flex items-center justify-between gap-4 border p-4">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{server.name}</span>
          <Badge variant="secondary">{server.source === "catalog" ? "Catalog" : "Custom"}</Badge>
          <SignInBadge server={server} />
        </div>
        <div className="truncate text-sm text-muted-foreground">
          {server.description ?? host(server.url)}
        </div>
        {update.error && <p className="text-sm text-destructive">{update.error.message}</p>}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {canConnect && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              window.location.href = connectUrl(identityId, provider);
            }}
          >
            <ExternalLink className="mr-1 h-4 w-4" />
            Sign in
          </Button>
        )}
        <Switch
          aria-label={server.enabled ? `Turn off ${server.name}` : `Turn on ${server.name}`}
          checked={server.enabled}
          disabled={update.isPending}
          onCheckedChange={(enabled) =>
            update.mutate({ serverId: server.id, request: { enabled } })
          }
        />
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          disabled={remove.isPending}
          onClick={() => {
            if (
              confirm(
                `Remove ${server.name}? Agents stop getting its tools and your sign-in is deleted.`,
              )
            ) {
              remove.mutate(server.id);
            }
          }}
        >
          <Trash2 className="mr-1 h-4 w-4" />
          Remove
        </Button>
      </div>
    </div>
  );
}

type AuthChoice = "none" | "oauth" | "api_key";

function AddServerDialog({
  identityId,
  open,
  onOpenChange,
}: {
  identityId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const policies = usePolicies("mcp-servers");
  const canBrowseCatalog = policies.can("mcp_server.view");
  const catalog = useMcpServerCatalog(open && canBrowseCatalog);
  const presets = (catalog.data ?? []).filter((preset) => preset.status === "active");
  const add = useAddUserMcpServer(identityId);
  const [mode, setMode] = useState<"catalog" | "custom">("catalog");
  const [preset, setPreset] = useState("");
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [auth, setAuth] = useState<AuthChoice>("oauth");
  const [apiKey, setApiKey] = useState("");

  useEffect(() => {
    if (open) {
      setPreset("");
      setName("");
      setUrl("");
      setAuth("oauth");
      setApiKey("");
      add.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const activeMode = canBrowseCatalog && presets.length > 0 ? mode : "custom";

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const request =
      activeMode === "catalog"
        ? { catalog: preset }
        : {
            name: name.trim(),
            url: url.trim(),
            auth_mode: auth,
            api_key: auth === "api_key" ? apiKey : undefined,
          };
    await add.mutateAsync(request);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>Add an MCP server</DialogTitle>
            <DialogDescription>
              Only you can use it. Agents that use your MCP servers get its tools when you chat with
              them.
            </DialogDescription>
          </DialogHeader>
          {canBrowseCatalog && presets.length > 0 && (
            <SectionTabs
              value={activeMode}
              onValueChange={(value) => setMode(value as "catalog" | "custom")}
              items={[
                { value: "catalog", label: "From catalog" },
                { value: "custom", label: "By URL" },
              ]}
            />
          )}
          {activeMode === "catalog" ? (
            <div className="space-y-2">
              <Label>Server</Label>
              <Select value={preset} onValueChange={setPreset}>
                <SelectTrigger aria-label="Catalog server" className="w-full">
                  <SelectValue placeholder="Choose a server" />
                </SelectTrigger>
                <SelectContent>
                  {presets.map((entry) => (
                    <SelectItem key={entry.id} value={entry.name}>
                      {entry.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          ) : (
            <>
              <div className="space-y-2">
                <Label htmlFor="user-mcp-name">Name</Label>
                <Input
                  id="user-mcp-name"
                  required
                  placeholder="notes"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="user-mcp-url">URL</Label>
                <Input
                  id="user-mcp-url"
                  required
                  type="url"
                  placeholder="https://mcp.example.com/mcp"
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label>Sign-in</Label>
                <Select value={auth} onValueChange={(value) => setAuth(value as AuthChoice)}>
                  <SelectTrigger aria-label="Sign-in" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="oauth">Sign in with the server (OAuth)</SelectItem>
                    <SelectItem value="api_key">API key</SelectItem>
                    <SelectItem value="none">None</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {auth === "api_key" && (
                <div className="space-y-2">
                  <Label htmlFor="user-mcp-key">API key</Label>
                  <Input
                    id="user-mcp-key"
                    required
                    type="password"
                    value={apiKey}
                    onChange={(event) => setApiKey(event.target.value)}
                  />
                </div>
              )}
            </>
          )}
          {add.error && <p className="text-sm text-destructive">{add.error.message}</p>}
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={add.isPending || (activeMode === "catalog" && !preset)}>
              Add server
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function UserMcpServersPanel({ identityId = "me" }: { identityId?: string }) {
  const { data: servers = [], isLoading, error } = useUserMcpServers(identityId);
  const [adding, setAdding] = useState(false);

  return (
    <section>
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h2 className="text-xl font-semibold">My MCP servers</h2>
          <p className="text-sm text-muted-foreground">
            MCP servers you added for yourself. Agents that use your MCP servers get their tools
            when you chat with them. Nobody else can use them.
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => setAdding(true)}>
          <Plus className="mr-1 h-4 w-4" />
          Add server
        </Button>
      </div>
      {error && (
        <div className="mb-4 bg-destructive/10 p-4 text-destructive">
          Failed to load your MCP servers: {error.message}
        </div>
      )}
      {isLoading ? (
        <Skeleton className="h-[72px] w-full" />
      ) : servers.length === 0 ? (
        <Card className="p-8 text-center">
          <McpIcon className="mx-auto mb-4 h-12 w-12 text-muted-foreground" />
          <h3 className="mb-2 text-lg font-medium">No MCP servers yet</h3>
          <p className="text-muted-foreground">
            Add a server from your organization&apos;s catalog or by URL.
          </p>
        </Card>
      ) : (
        <div className="space-y-2">
          {servers.map((server) => (
            <ServerRow key={server.id} identityId={identityId} server={server} />
          ))}
        </div>
      )}
      <AddServerDialog identityId={identityId} open={adding} onOpenChange={setAdding} />
    </section>
  );
}
