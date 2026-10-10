"use client";

import { useId, useMemo, useState } from "react";
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
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  useAgentMcpAttachments,
  useRevokeAgentMcpConnection,
  useUpdateAgent,
} from "@/hooks/use-agents";
import { useMcpServers } from "@/hooks/use-mcp-servers";
import { AgentUserMcpGroup } from "@/components/agents/agent-user-mcp-group";
import { ConnectErrorBanner } from "@/components/connections/connect-error-banner";
import type { Agent, AgentMcpAttachment, McpServerActsAs, ScopedMcpServers } from "@/lib/api/types";
function attachmentPrefix(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]/g, "_");
}

function presetHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function presetAuthLabel(authMode: string): string {
  if (authMode === "oauth") return "OAuth";
  if (authMode === "api_key") return "API key";
  return "No auth";
}

function identityLabel(actsAs: McpServerActsAs): string {
  switch (actsAs) {
    case "service":
      return "Service identity";
    case "user":
      return "Invoking user";
    case "user_or_service":
      return "Acts as: the user, or the agent when the user has not connected";
    default:
      return "No identity";
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "The request failed. Try again.";
}
function connectionHref(agentId: string, attachment: AgentMcpAttachment): string | null {
  if (!attachment.connection_provider) return null;
  // The action says whose login is missing: `authorize` is the agent's
  // (service) login, `connect` the caller's own. A `user_or_service`
  // attachment can need either, so the identity alone does not decide it.
  const forAgent =
    attachment.action === "authorize" ||
    (attachment.action !== "connect" && attachment.acts_as === "service");
  // A connection-backed preset signs the agent in through its own provider
  // connection (the agent's GitHub App), not a separate MCP login.
  if (forAgent && attachment.service_connection_provider) {
    return `/agents/${agentId}?tab=integrations`;
  }
  const params = new URLSearchParams({
    return_to: `/agents/${agentId}?tab=mcp`,
    mode: forAgent ? "identity" : "user",
  });
  if (forAgent) params.set("agent_id", agentId);
  return `/api/v1/user/connections/${encodeURIComponent(attachment.connection_provider)}/authorize?${params}`;
}

function McpAttachmentRow({
  agentId,
  attachment,
  hasCollision,
  onRemove,
  onConnectInChatChange,
  onDeferredChange,
  saving,
}: {
  agentId: string;
  attachment: AgentMcpAttachment;
  hasCollision: boolean;
  onRemove: (attachment: AgentMcpAttachment) => void;
  onConnectInChatChange: (attachment: AgentMcpAttachment, allow: boolean) => void;
  onDeferredChange: (attachment: AgentMcpAttachment, deferred: boolean) => void;
  saving: boolean;
}) {
  const toolsId = useId();
  const connectInChatId = useId();
  const deferredId = useId();
  const connectsInChat = attachment.connect_in_chat !== "never";
  const [toolsOpen, setToolsOpen] = useState(false);
  const revoke = useRevokeAgentMcpConnection(agentId);
  const connectHref = connectionHref(agentId, attachment);
  const isCapability = attachment.source === "capability";

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <CardTitle className="flex flex-wrap items-center gap-2">
            <span className="break-all">{attachment.name}</span>
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
          {hasCollision && (
            <p className="flex items-center gap-1 text-xs text-destructive">
              <AlertTriangle className="size-3" />
              Tool prefix collides with another attachment name.
            </p>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {attachment.contributor && (
            <LinkButton href={attachment.contributor.href} variant="outline" size="sm">
              View {attachment.contributor.name}
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
        {attachment.acts_as !== "none" &&
          (attachment.editable ? (
            <div className="flex items-start gap-3">
              <Switch
                id={connectInChatId}
                checked={connectsInChat}
                disabled={saving}
                aria-label="Ask to connect in chat"
                onCheckedChange={(allow) => onConnectInChatChange(attachment, allow)}
              />
              <div className="space-y-0.5">
                <Label htmlFor={connectInChatId}>Ask to connect in chat</Label>
                <p className="text-xs text-muted-foreground">
                  {connectsInChat
                    ? "A missing sign-in pauses the chat with a Connect card."
                    : "A missing sign-in fails the call with a link to settings. For channels that cannot show cards."}
                </p>
              </div>
            </div>
          ) : (
            !connectsInChat && <Badge variant="outline">Connects in settings only</Badge>
          ))}
        {attachment.editable ? (
          <div className="flex items-start gap-3">
            <Switch
              id={deferredId}
              checked={attachment.deferred}
              disabled={saving}
              aria-label="Load tools on demand"
              onCheckedChange={(deferred) => onDeferredChange(attachment, deferred)}
            />
            <div className="space-y-0.5">
              <Label htmlFor={deferredId}>Load tools on demand</Label>
              <p className="text-xs text-muted-foreground">
                {attachment.deferred
                  ? "The agent sees one line for this server and loads its tools through tool search when it needs them."
                  : "The server's tools are listed at the start of every turn."}
              </p>
            </div>
          </div>
        ) : (
          attachment.deferred && <Badge variant="outline">Tools load on demand</Badge>
        )}

        {attachment.state === "preset_missing" ? (
          <div className="flex flex-wrap items-center gap-2 text-sm text-destructive">
            <span className="flex items-center gap-2">
              <AlertTriangle className="size-4" />
              This preset is no longer available.
            </span>
            <LinkButton href="/settings/mcp-catalog" variant="outline" size="sm">
              View catalog
            </LinkButton>
          </div>
        ) : attachment.state === "connection_missing" ? (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm text-muted-foreground">Connection required.</span>
            {isCapability ? null : attachment.action === "ask_admin" ? (
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
            {!isCapability &&
              (attachment.action === "connect" || attachment.action === "authorize") &&
              connectHref && (
                // `user_or_service` with one of its two logins in place: the
                // caller can still add their own account (which then takes
                // precedence), or a manager the agent's fallback login.
                <LinkButton href={connectHref} variant="outline" size="sm">
                  <Link2 className="size-4" />
                  {attachment.action === "authorize"
                    ? "Authorize the agent"
                    : "Connect your account"}
                </LinkButton>
              )}
            {attachment.can_revoke ? (
              <Button
                variant="outline"
                size="sm"
                disabled={revoke.isPending}
                onClick={() => revoke.mutate(attachment.name)}
              >
                <Unplug className="size-4" />
                Revoke
              </Button>
            ) : !isCapability &&
              !attachment.service_connection_provider &&
              // Not revocable here means the agent's login is the one in use
              // and only an admin may change it.
              (attachment.acts_as === "service" || attachment.acts_as === "user_or_service") ? (
              <Badge variant="outline">Ask an admin</Badge>
            ) : null}
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
            {!attachment.tools_available && (
              <span className="text-xs text-muted-foreground">Unavailable</span>
            )}
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
  const [connectInChat, setConnectInChat] = useState(true);
  const [rowError, setRowError] = useState<string | null>(null);
  const [customName, setCustomName] = useState("");
  const [customUrl, setCustomUrl] = useState("");
  const [customHeaders, setCustomHeaders] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const existingAttachmentNames = useMemo(
    () =>
      new Set([
        ...Object.keys(agent.mcpServers ?? {}),
        ...(attachments.data ?? []).map((attachment) => attachment.name),
      ]),
    [agent.mcpServers, attachments.data],
  );

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
  const prefixCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const attachment of attachments.data ?? []) {
      const prefix = attachmentPrefix(attachment.name);
      counts.set(prefix, (counts.get(prefix) ?? 0) + 1);
    }
    return counts;
  }, [attachments.data]);

  const saveAuthoredAttachments = async (mcpServers: ScopedMcpServers) => {
    await updateAgent.mutateAsync({ agentId: agent.id, request: { mcpServers } });
    await attachments.refetch();
  };

  const closeAdd = () => {
    setAddOpen(false);
    setSearch("");
    setSelectedPreset(null);
    setActsAs("none");
    setConnectInChat(true);
    setCustomName("");
    setCustomUrl("");
    setCustomHeaders("");
    setAddError(null);
  };

  const addPreset = async () => {
    if (!selectedPreset) return;
    setAddError(null);
    if (existingAttachmentNames.has(selectedPreset)) {
      setAddError(`An MCP attachment named “${selectedPreset}” already exists.`);
      return;
    }
    try {
      await saveAuthoredAttachments({
        ...(agent.mcpServers ?? {}),
        [selectedPreset]: {
          use: `catalog:${selectedPreset}`,
          actsAs,
          // `ask` is the default and stays off the wire.
          ...(actsAs !== "none" && !connectInChat ? { connectInChat: "never" as const } : {}),
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
    if (existingAttachmentNames.has(name)) {
      setAddError(`An MCP attachment named “${name}” already exists.`);
      return;
    }
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

  const changeConnectInChat = async (attachment: AgentMcpAttachment, allow: boolean) => {
    const current = agent.mcpServers?.[attachment.name];
    if (!current) return;
    setRowError(null);
    const rest = { ...current };
    delete rest.connectInChat;
    try {
      await saveAuthoredAttachments({
        ...(agent.mcpServers ?? {}),
        [attachment.name]: allow ? rest : { ...rest, connectInChat: "never" },
      });
    } catch (error) {
      setRowError(errorMessage(error));
    }
  };

  const changeDeferred = async (attachment: AgentMcpAttachment, deferred: boolean) => {
    const current = agent.mcpServers?.[attachment.name];
    if (!current) return;
    setRowError(null);
    const rest = { ...current };
    delete rest.deferred;
    try {
      await saveAuthoredAttachments({
        ...(agent.mcpServers ?? {}),
        // Off is the default and stays off the wire.
        [attachment.name]: deferred ? { ...rest, deferred: true } : rest,
      });
    } catch (error) {
      setRowError(errorMessage(error));
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
      {/* Authorize / Connect return here (`?tab=mcp`); a failure carries `connect_error`. */}
      <ConnectErrorBanner />
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
              hasCollision={(prefixCounts.get(attachmentPrefix(attachment.name)) ?? 0) > 1}
              onRemove={setRemoveTarget}
              onConnectInChatChange={changeConnectInChat}
              onDeferredChange={changeDeferred}
              saving={updateAgent.isPending}
            />
          ))}
          {rowError && <p className="text-sm text-destructive">{rowError}</p>}
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

      <AgentUserMcpGroup agent={agent} attachments={attachments.data ?? []} />

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
                  visiblePresets.map((preset) => {
                    const alreadyAttached = existingAttachmentNames.has(preset.name);
                    return (
                      <button
                        type="button"
                        key={preset.id}
                        disabled={alreadyAttached}
                        className={`w-full border p-3 text-left disabled:cursor-not-allowed disabled:opacity-60 ${
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
                        <span className="flex items-center justify-between gap-2 text-sm font-medium">
                          {preset.name}
                          {alreadyAttached && <Badge variant="secondary">Already attached</Badge>}
                        </span>
                        {preset.description && (
                          <span className="block text-xs text-muted-foreground">
                            {preset.description}
                          </span>
                        )}
                        <span className="mt-1 block text-xs text-muted-foreground">
                          {presetHost(preset.url)} · {presetAuthLabel(preset.auth_mode)}
                        </span>
                      </button>
                    );
                  })
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
                <Label>
                  <input
                    type="radio"
                    name="mcp-acts-as"
                    value="user_or_service"
                    checked={actsAs === "user_or_service"}
                    disabled={identityDisabled}
                    onChange={() => setActsAs("user_or_service")}
                  />
                  Each user, or the agent if they have not connected
                </Label>
                {selectedPresetRecord && identityDisabled && (
                  <p className="text-xs text-muted-foreground">
                    This preset does not support OAuth identity grants.
                  </p>
                )}
              </fieldset>
              {actsAs !== "none" && (
                <Label>
                  <input
                    type="checkbox"
                    checked={connectInChat}
                    onChange={(event) => setConnectInChat(event.target.checked)}
                  />
                  Ask to connect in chat when a sign-in is missing
                </Label>
              )}
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
              Remove {removeTarget?.name} from this agent. New sessions will no longer be able to
              use it. Existing user and service grants are not revoked.
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
