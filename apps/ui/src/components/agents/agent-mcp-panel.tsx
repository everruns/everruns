"use client";

import { useMemo, useState } from "react";
import { AlertTriangle, ChevronDown, Link2, Plus, Trash2, Unplug, Wrench } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { Textarea } from "@/components/ui/textarea";
import {
  useAgentMcpAttachments,
  useRevokeAgentMcpConnection,
  useUpdateAgent,
} from "@/hooks/use-agents";
import { useMcpServers } from "@/hooks/use-mcp-servers";
import type { Agent, AgentMcpAttachment, McpServerActsAs, ScopedMcpServers } from "@/lib/api/types";

function identityLabel(actsAs: McpServerActsAs): string {
  switch (actsAs) {
    case "service":
      return "Service identity";
    case "user":
      return "Invoking user";
    default:
      return "No identity";
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "The request failed. Try again.";
}
function connectionHref(agentId: string, attachment: AgentMcpAttachment): string | null {
  if (!attachment.connection_provider) return null;
  const params = new URLSearchParams({
    return_to: `/agents/${agentId}?tab=mcp`,
    mode: attachment.acts_as === "service" ? "identity" : "user",
  });
  if (attachment.acts_as === "service") params.set("agent_id", agentId);
  return `/api/v1/user/connections/${encodeURIComponent(attachment.connection_provider)}/authorize?${params}`;
}

function McpAttachmentRow({
  agentId,
  attachment,
  onRemove,
}: {
  agentId: string;
  attachment: AgentMcpAttachment;
  onRemove: (attachment: AgentMcpAttachment) => void;
}) {
  const [toolsOpen, setToolsOpen] = useState(false);
  const revoke = useRevokeAgentMcpConnection(agentId);
  const connectHref = connectionHref(agentId, attachment);
  const toolsId = `mcp-tools-${attachment.name.replace(/[^a-zA-Z0-9_-]/g, "-")}`;

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <CardTitle className="flex flex-wrap items-center gap-2">
            <span>{attachment.name}</span>
            <Badge variant="outline">{identityLabel(attachment.acts_as)}</Badge>
          </CardTitle>
          <p className="text-sm text-muted-foreground">
            Source: <span className="text-foreground">{attachment.source_label}</span>
          </p>
          {attachment.overridden_sources.length > 0 && (
            <p className="text-xs text-muted-foreground">
              Overrides:{" "}
              {attachment.overridden_sources.map((source) => source.source_label).join(", ")}
            </p>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {attachment.source === "capability" && (
            <LinkButton href="/capabilities" variant="outline" size="sm">
              View capability
            </LinkButton>
          )}
          {attachment.editable && (
            <Button variant="outline" size="sm" onClick={() => onRemove(attachment)}>
              <Trash2 className="size-4" />
              Remove
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {attachment.url && <p className="break-all font-mono text-xs">{attachment.url}</p>}
        {attachment.preset_name && (
          <p className="text-sm text-muted-foreground">
            Preset: <span className="text-foreground">{attachment.preset_name}</span>
          </p>
        )}
        {attachment.header_names.length > 0 && (
          <p className="text-sm text-muted-foreground">
            Headers:{" "}
            <span className="font-mono text-foreground">{attachment.header_names.join(", ")}</span>
          </p>
        )}

        {attachment.state === "preset_missing" ? (
          <div className="flex items-center gap-2 text-sm text-destructive">
            <AlertTriangle className="size-4" />
            This preset is no longer available.
          </div>
        ) : attachment.state === "connection_missing" ? (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm text-muted-foreground">Connection required.</span>
            {attachment.action === "ask_admin" ? (
              <Badge variant="outline">Ask an admin</Badge>
            ) : connectHref ? (
              <LinkButton href={connectHref} size="sm">
                <Link2 className="size-4" />
                {attachment.action === "authorize" ? "Authorize" : "Connect"}
              </LinkButton>
            ) : null}
          </div>
        ) : attachment.connected_as ? (
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="success">Connected as {attachment.connected_as}</Badge>
            <Button
              variant="outline"
              size="sm"
              disabled={revoke.isPending}
              onClick={() => revoke.mutate(attachment.name)}
            >
              <Unplug className="size-4" />
              Revoke
            </Button>
            {revoke.error && (
              <span className="text-sm text-destructive">{errorMessage(revoke.error)}</span>
            )}
          </div>
        ) : (
          <Badge variant="success">Ready</Badge>
        )}

        <div>
          <Button
            variant="ghost"
            size="sm"
            aria-expanded={toolsOpen}
            aria-controls={toolsId}
            onClick={() => setToolsOpen((open) => !open)}
          >
            <Wrench className="size-4" />
            Tools
            <Badge variant="secondary">{attachment.tools.length}</Badge>
            <ChevronDown
              className={`size-4 transition-transform ${toolsOpen ? "rotate-180" : ""}`}
            />
          </Button>
          {toolsOpen && (
            <div id={toolsId} className="mt-2 border bg-muted/30 p-3">
              {attachment.tools.length > 0 ? (
                <ul className="space-y-1">
                  {attachment.tools.map((tool) => (
                    <li key={tool} className="font-mono text-xs">
                      {tool}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">No cached tools are available.</p>
              )}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

export function AgentMcpPanel({ agent }: { agent: Agent }) {
  const attachments = useAgentMcpAttachments(agent.id);
  const updateAgent = useUpdateAgent();
  const { data: presets = [], isLoading: presetsLoading } = useMcpServers();
  const [addOpen, setAddOpen] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<AgentMcpAttachment | null>(null);
  const [flow, setFlow] = useState<"preset" | "custom">("preset");
  const [search, setSearch] = useState("");
  const [selectedPreset, setSelectedPreset] = useState<string | null>(null);
  const [actsAs, setActsAs] = useState<McpServerActsAs>("none");
  const [customName, setCustomName] = useState("");
  const [customUrl, setCustomUrl] = useState("");
  const [customHeaders, setCustomHeaders] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [removeError, setRemoveError] = useState<string | null>(null);

  const visiblePresets = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return presets.filter(
      (preset) =>
        preset.status === "active" &&
        (!needle ||
          preset.name.toLowerCase().includes(needle) ||
          preset.description?.toLowerCase().includes(needle)),
    );
  }, [presets, search]);
  const selectedPresetRecord = presets.find((preset) => preset.name === selectedPreset);
  const identityDisabled = selectedPresetRecord?.auth_mode !== "oauth";

  const saveAuthoredAttachments = async (mcpServers: ScopedMcpServers) => {
    await updateAgent.mutateAsync({ agentId: agent.id, request: { mcpServers } });
    await attachments.refetch();
  };

  const closeAdd = () => {
    setAddOpen(false);
    setSearch("");
    setSelectedPreset(null);
    setActsAs("none");
    setCustomName("");
    setCustomUrl("");
    setCustomHeaders("");
    setAddError(null);
  };

  const addPreset = async () => {
    if (!selectedPreset) return;
    setAddError(null);
    try {
      await saveAuthoredAttachments({
        ...(agent.mcpServers ?? {}),
        [selectedPreset]: {
          use: `catalog:${selectedPreset}`,
          actsAs,
        },
      });
      closeAdd();
    } catch (error) {
      setAddError(errorMessage(error));
    }
  };

  const addCustom = async () => {
    const name = customName.trim();
    const url = customUrl.trim();
    if (!name || !url) return;
    setAddError(null);
    try {
      const parsedUrl = new URL(url);
      if (!["http:", "https:"].includes(parsedUrl.protocol)) {
        throw new Error("URL must use HTTP or HTTPS.");
      }
    } catch (error) {
      setAddError(
        error instanceof Error && error.message === "URL must use HTTP or HTTPS."
          ? error.message
          : "URL must be a valid absolute URL.",
      );
      return;
    }
    const headerLines = customHeaders
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    const headers = Object.fromEntries(
      headerLines
        .map((line) => {
          const separator = line.indexOf(":");
          return separator === -1
            ? ["", ""]
            : [line.slice(0, separator).trim(), line.slice(separator + 1).trim()];
        })
        .filter(([key, value]) => key && value),
    );
    if (Object.keys(headers).length !== headerLines.length) {
      setAddError("Each header must use the format “Name: value”.");
      return;
    }
    try {
      await saveAuthoredAttachments({
        ...(agent.mcpServers ?? {}),
        [name]: {
          type: "http",
          url,
          ...(Object.keys(headers).length > 0 ? { headers } : {}),
          actsAs: "none",
        },
      });
      closeAdd();
    } catch (error) {
      setAddError(errorMessage(error));
    }
  };

  const removeAttachment = async () => {
    if (!removeTarget) return;
    setRemoveError(null);
    const next = { ...(agent.mcpServers ?? {}) };
    delete next[removeTarget.name];
    try {
      await saveAuthoredAttachments(next);
      setRemoveTarget(null);
    } catch (error) {
      setRemoveError(errorMessage(error));
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold">MCP attachments</h2>
          <p className="text-sm text-muted-foreground">
            Effective servers from capabilities, the harness, and this agent.
          </p>
        </div>
        <Button onClick={() => setAddOpen(true)}>
          <Plus className="size-4" />
          Add MCP server
        </Button>
      </div>

      {attachments.isLoading ? (
        <Card>
          <CardContent className="py-8 text-center text-sm text-muted-foreground">
            Loading MCP attachments…
          </CardContent>
        </Card>
      ) : attachments.error ? (
        <Card>
          <CardContent className="py-8 text-center text-sm text-destructive">
            Failed to load MCP attachments.
          </CardContent>
        </Card>
      ) : attachments.data?.length ? (
        <div className="space-y-3">
          {attachments.data.map((attachment) => (
            <McpAttachmentRow
              key={attachment.name}
              agentId={agent.id}
              attachment={attachment}
              onRemove={setRemoveTarget}
            />
          ))}
        </div>
      ) : (
        <Card>
          <CardContent className="py-8 text-center">
            <p className="font-medium">No MCP servers attached</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Add an organization preset or a custom HTTP server.
            </p>
          </CardContent>
        </Card>
      )}

      <Dialog open={addOpen} onOpenChange={(open) => (open ? setAddOpen(true) : closeAdd())}>
        <DialogContent className="sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>Add MCP server</DialogTitle>
            <DialogDescription>
              Attach an organization preset or enter a custom HTTP endpoint.
            </DialogDescription>
          </DialogHeader>
          <div className="flex gap-2">
            <Button
              type="button"
              variant={flow === "preset" ? "default" : "outline"}
              aria-pressed={flow === "preset"}
              onClick={() => {
                setFlow("preset");
                setAddError(null);
              }}
            >
              Preset
            </Button>
            <Button
              type="button"
              variant={flow === "custom" ? "default" : "outline"}
              aria-pressed={flow === "custom"}
              onClick={() => {
                setFlow("custom");
                setAddError(null);
              }}
            >
              Custom
            </Button>
          </div>

          {flow === "preset" ? (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="mcp-preset-search">Search presets</Label>
                <Input
                  id="mcp-preset-search"
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder="Search by name or description"
                />
              </div>
              <div className="max-h-52 space-y-2 overflow-y-auto">
                {presetsLoading ? (
                  <p className="text-sm text-muted-foreground">Loading presets…</p>
                ) : visiblePresets.length ? (
                  visiblePresets.map((preset) => (
                    <button
                      type="button"
                      key={preset.id}
                      className={`w-full border p-3 text-left ${
                        selectedPreset === preset.name
                          ? "border-primary bg-muted"
                          : "hover:bg-muted/50"
                      }`}
                      aria-pressed={selectedPreset === preset.name}
                      onClick={() => {
                        setSelectedPreset(preset.name);
                        setActsAs(preset.auth_mode === "oauth" ? "service" : "none");
                      }}
                    >
                      <span className="block text-sm font-medium">{preset.name}</span>
                      {preset.description && (
                        <span className="block text-xs text-muted-foreground">
                          {preset.description}
                        </span>
                      )}
                    </button>
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">No matching presets.</p>
                )}
              </div>
              <fieldset className="space-y-2">
                <legend className="text-sm font-medium">Who should this server act as?</legend>
                <Label>
                  <input
                    type="radio"
                    name="mcp-acts-as"
                    value="none"
                    checked={actsAs === "none"}
                    onChange={() => setActsAs("none")}
                  />
                  No identity
                </Label>
                <Label>
                  <input
                    type="radio"
                    name="mcp-acts-as"
                    value="service"
                    checked={actsAs === "service"}
                    disabled={identityDisabled}
                    onChange={() => setActsAs("service")}
                  />
                  Service identity
                </Label>
                <Label>
                  <input
                    type="radio"
                    name="mcp-acts-as"
                    value="user"
                    checked={actsAs === "user"}
                    disabled={identityDisabled}
                    onChange={() => setActsAs("user")}
                  />
                  Invoking user
                </Label>
                {selectedPresetRecord && identityDisabled && (
                  <p className="text-xs text-muted-foreground">
                    This preset does not support OAuth identity grants.
                  </p>
                )}
              </fieldset>
            </div>
          ) : (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="mcp-custom-name">Name</Label>
                <Input
                  id="mcp-custom-name"
                  value={customName}
                  onChange={(event) => setCustomName(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="mcp-custom-url">URL</Label>
                <Input
                  id="mcp-custom-url"
                  type="url"
                  value={customUrl}
                  onChange={(event) => setCustomUrl(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="mcp-custom-headers">Headers</Label>
                <Textarea
                  id="mcp-custom-headers"
                  value={customHeaders}
                  onChange={(event) => setCustomHeaders(event.target.value)}
                  placeholder={"Header-Name: value\nAnother-Header: value"}
                />
                <p className="text-xs text-muted-foreground">
                  Custom servers do not use a user or service grant.
                </p>
              </div>
            </div>
          )}

          {addError && <p className="text-sm text-destructive">{addError}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={closeAdd}>
              Cancel
            </Button>
            <Button
              disabled={
                updateAgent.isPending ||
                (flow === "preset" ? !selectedPreset : !customName.trim() || !customUrl.trim())
              }
              onClick={flow === "preset" ? addPreset : addCustom}
            >
              Add server
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={!!removeTarget}
        onOpenChange={(open) => {
          if (!open) {
            setRemoveTarget(null);
            setRemoveError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove MCP attachment?</DialogTitle>
            <DialogDescription>
              Remove {removeTarget?.name} from this agent. Existing user and service grants are not
              revoked.
            </DialogDescription>
          </DialogHeader>
          {removeError && <p className="text-sm text-destructive">{removeError}</p>}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setRemoveTarget(null);
                setRemoveError(null);
              }}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={updateAgent.isPending}
              onClick={removeAttachment}
            >
              Remove attachment
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
