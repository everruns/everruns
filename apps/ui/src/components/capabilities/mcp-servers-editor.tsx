"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Plug, Plus, Trash2 } from "lucide-react";
import { KeyValueEditor } from "./key-value-editor";
import type { McpServerEntry } from "./declarative-form-types";
import { newMcpServerEntry } from "./declarative-form-types";

const MAX_MCP_SERVERS = 16;

export function McpServersEditor({
  servers,
  onChange,
}: {
  servers: McpServerEntry[];
  onChange: (servers: McpServerEntry[]) => void;
}) {
  const update = (id: string, patch: Partial<McpServerEntry>) =>
    onChange(servers.map((s) => (s.id === id ? { ...s, ...patch } : s)));

  return (
    <div className="space-y-4">
      {servers.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No MCP servers. Add one to expose its tools to agents using this capability.
        </p>
      )}
      {servers.map((server) => (
        <div key={server.id} className="space-y-4 border p-4">
          <div className="flex items-start justify-between gap-2">
            <div className="flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center bg-primary/10">
                <Plug className="h-4 w-4 text-primary" />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor={`mcp-name-${server.id}`}>Server name</Label>
                <Input
                  id={`mcp-name-${server.id}`}
                  placeholder="atlassian"
                  value={server.name}
                  onChange={(e) => update(server.id, { name: e.target.value })}
                  className="w-64"
                />
              </div>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-9 w-9 p-0 shrink-0"
              onClick={() => onChange(servers.filter((s) => s.id !== server.id))}
              aria-label="Remove server"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>

          <div className="grid gap-1.5">
            <Label>Transport</Label>
            <Select
              value={server.transportType}
              onValueChange={(value) =>
                update(server.id, { transportType: value as McpServerEntry["transportType"] })
              }
            >
              <SelectTrigger className="w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="http">HTTP (remote)</SelectItem>
                <SelectItem value="stdio">stdio (local process)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {server.transportType === "http" ? (
            <>
              <div className="grid gap-1.5">
                <Label htmlFor={`mcp-url-${server.id}`}>URL</Label>
                <Input
                  id={`mcp-url-${server.id}`}
                  placeholder="https://mcp.example.com/v1/mcp"
                  value={server.url}
                  onChange={(e) => update(server.id, { url: e.target.value })}
                />
              </div>
              <KeyValueEditor
                label="Headers"
                pairs={server.headers}
                onChange={(headers) => update(server.id, { headers })}
                addLabel="Add header"
              />
            </>
          ) : (
            <>
              <div className="grid gap-1.5">
                <Label htmlFor={`mcp-command-${server.id}`}>Command</Label>
                <Input
                  id={`mcp-command-${server.id}`}
                  placeholder="npx"
                  value={server.command}
                  onChange={(e) => update(server.id, { command: e.target.value })}
                />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor={`mcp-args-${server.id}`}>Arguments (one per line)</Label>
                <Textarea
                  id={`mcp-args-${server.id}`}
                  placeholder={"-y\n@modelcontextprotocol/server-filesystem"}
                  value={server.args}
                  onChange={(e) => update(server.id, { args: e.target.value })}
                  rows={3}
                  className="font-mono text-xs"
                />
              </div>
              <KeyValueEditor
                label="Environment variables"
                pairs={server.env}
                onChange={(env) => update(server.id, { env })}
                addLabel="Add variable"
              />
            </>
          )}

          <Separator />

          <div className="grid gap-1.5">
            <Label>Authentication</Label>
            <Select
              value={server.authMode}
              onValueChange={(value) =>
                update(server.id, { authMode: value as McpServerEntry["authMode"] })
              }
            >
              <SelectTrigger className="w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                <SelectItem value="api_key">API key</SelectItem>
                <SelectItem value="oauth">OAuth per user</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {server.authMode === "oauth" && (
            <div className="grid gap-1.5">
              <Label htmlFor={`mcp-oauth-${server.id}`}>OAuth provider ID</Label>
              <Input
                id={`mcp-oauth-${server.id}`}
                placeholder="atlassian"
                value={server.oauthProviderId}
                onChange={(e) => update(server.id, { oauthProviderId: e.target.value })}
              />
            </div>
          )}

          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label htmlFor={`mcp-discovery-${server.id}`}>Live tool discovery</Label>
              <p className="text-xs text-muted-foreground">
                Discover tool definitions from the server at runtime.
              </p>
            </div>
            <Switch
              id={`mcp-discovery-${server.id}`}
              checked={server.toolDiscovery}
              onCheckedChange={(checked) => update(server.id, { toolDiscovery: checked })}
            />
          </div>
        </div>
      ))}

      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={servers.length >= MAX_MCP_SERVERS}
        onClick={() => onChange([...servers, newMcpServerEntry()])}
      >
        <Plus className="h-4 w-4 mr-1" />
        Add MCP server
      </Button>
    </div>
  );
}
