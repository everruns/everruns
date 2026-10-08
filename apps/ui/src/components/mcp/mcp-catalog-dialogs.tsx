"use client";

// Create, edit, key, header and archive dialogs for the org MCP catalog
// (Settings > Organization > MCP catalog).

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
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
  useMcpServerUsage,
  useCreateMcpServer,
  useDeleteMcpServer,
  useUpdateMcpServer,
} from "@/hooks/use-mcp-servers";
import { Plus, X } from "lucide-react";
import type {
  McpServer,
  CreateMcpServerRequest,
  McpServerAuthMode,
  McpProtocolMode,
  McpElicitationPolicy,
} from "@/lib/api/types";
import {
  apiKeySecretSchema,
  getFieldErrors,
  mcpServerEditFormSchema,
  type McpServiceConnectionChoice,
  mcpServerFormSchema,
  type FieldErrors,
} from "@/lib/form-validation";
import { pluralize } from "@/lib/formatting";
import { ElicitationPolicyField, ServiceConnectionField } from "@/components/mcp/mcp-server-fields";

export function AddMcpServerDialog({
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
  const [protocolMode, setProtocolMode] = useState<McpProtocolMode>("auto");
  const [elicitationPolicy, setElicitationPolicy] = useState<McpElicitationPolicy>("url");
  const [serviceConnection, setServiceConnection] = useState<McpServiceConnectionChoice>("none");
  const [headers, setHeaders] = useState<Array<{ key: string; value: string }>>([]);
  const [headerErrors, setHeaderErrors] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  const createServer = useCreateMcpServer();

  useEffect(() => {
    if (!open) return;
    setName("");
    setDescription("");
    setUrl("");
    setApiKey("");
    setAuthMode("none");
    setProtocolMode("auto");
    setElicitationPolicy("url");
    setServiceConnection("none");
    setHeaders([]);
    setHeaderErrors(null);
    setFormError(null);
    setFieldErrors({});
  }, [open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setFormError(null);
    const parsed = mcpServerFormSchema.safeParse({
      name,
      description,
      url,
      auth_mode: authMode,
      api_key: apiKey,
      service_connection_provider: serviceConnection,
    });
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    const nonEmptyHeaders = headers.filter((h) => h.key || h.value);
    const missingKey = nonEmptyHeaders.find((h) => !h.key);
    if (missingKey) {
      setHeaderErrors("All headers must have a name");
      return;
    }
    setHeaderErrors(null);
    const headersRecord =
      nonEmptyHeaders.length > 0
        ? Object.fromEntries(nonEmptyHeaders.map((h) => [h.key, h.value]))
        : undefined;

    const data: CreateMcpServerRequest = {
      name: parsed.data.name,
      description: parsed.data.description,
      url: parsed.data.url,
      transport_type: "http",
      auth_mode: parsed.data.auth_mode,
      protocol_mode: protocolMode === "auto" ? undefined : protocolMode,
      elicitation_policy: elicitationPolicy === "url" ? undefined : elicitationPolicy,
      service_connection_provider:
        parsed.data.service_connection_provider === "none"
          ? undefined
          : parsed.data.service_connection_provider,
      api_key: parsed.data.auth_mode === "api_key" ? parsed.data.api_key : undefined,
      headers: headersRecord,
    };
    try {
      await createServer.mutateAsync(data);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "MCP server could not be created");
      return;
    }
    onOpenChange(false);
    setName("");
    setDescription("");
    setUrl("");
    setApiKey("");
    setAuthMode("none");
    setProtocolMode("auto");
    setElicitationPolicy("url");
    setHeaders([]);
    setHeaderErrors(null);
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
                setFieldErrors((prev) => ({
                  ...prev,
                  auth_mode: undefined,
                  api_key: undefined,
                }));
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
          <div className="space-y-2">
            <Label htmlFor="protocol-mode">Protocol compatibility</Label>
            <Select
              value={protocolMode}
              onValueChange={(value) => setProtocolMode(value as McpProtocolMode)}
            >
              <SelectTrigger id="protocol-mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auto">Auto (negotiate)</SelectItem>
                <SelectItem value="2026-07-28">2026-07-28 — stateless</SelectItem>
                <SelectItem value="2025-06-18">2025-06-18 — stateful</SelectItem>
                <SelectItem value="2025-03-26">2025-03-26 — stateful</SelectItem>
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              Auto probes the server and adapts across every protocol era. Pin a version only to
              work around a server that mis-signals its era.
            </p>
          </div>
          <ElicitationPolicyField
            id="elicitation-policy"
            value={elicitationPolicy}
            onChange={setElicitationPolicy}
          />
          <ServiceConnectionField
            id="service-connection"
            value={serviceConnection}
            onChange={setServiceConnection}
          />
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
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label>Custom Headers (optional)</Label>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                onClick={() => setHeaders((prev) => [...prev, { key: "", value: "" }])}
              >
                <Plus className="h-3 w-3 mr-1" />
                Add header
              </Button>
            </div>
            {headers.length > 0 && (
              <div className="space-y-2">
                {headers.map((header, index) => (
                  <div key={index} className="flex items-center gap-2">
                    <Input
                      placeholder="Name"
                      value={header.key}
                      onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                        setHeaders((prev) =>
                          prev.map((h, i) => (i === index ? { ...h, key: e.target.value } : h)),
                        );
                        setHeaderErrors(null);
                      }}
                      className="flex-1"
                    />
                    <Input
                      placeholder="Value"
                      value={header.value}
                      onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                        setHeaders((prev) =>
                          prev.map((h, i) => (i === index ? { ...h, value: e.target.value } : h)),
                        );
                      }}
                      className="flex-1"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-9 w-9 p-0 shrink-0"
                      onClick={() => setHeaders((prev) => prev.filter((_, i) => i !== index))}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
            {headerErrors && <p className="text-xs text-destructive">{headerErrors}</p>}
          </div>
          {formError && (
            <p role="alert" className="text-sm text-destructive">
              {formError}
            </p>
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

function urlOrigin(value: string): string | null {
  try {
    return new URL(value).origin;
  } catch {
    return null;
  }
}

export function EditMcpServerDialog({
  server,
  open,
  onOpenChange,
}: {
  server: McpServer | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [url, setUrl] = useState("");
  const [protocolMode, setProtocolMode] = useState<McpProtocolMode>("auto");
  const [elicitationPolicy, setElicitationPolicy] = useState<McpElicitationPolicy>("url");
  const [serviceConnection, setServiceConnection] = useState<McpServiceConnectionChoice>("none");
  const [apiKey, setApiKey] = useState("");
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  const updateServer = useUpdateMcpServer(server?.id ?? "");

  // Stored credentials are bound to the server's origin (EVE-1192): the API
  // rejects moving a server to another origin while it would keep its API key
  // or custom headers. Ask for a fresh key and drop headers explicitly.
  const originChanged = !!server && urlOrigin(url) !== urlOrigin(server.url);
  const needsFreshApiKey = originChanged && server?.auth_mode === "api_key" && !!server.api_key_set;
  const clearsHeaders = originChanged && Object.keys(server?.headers ?? {}).length > 0;

  // Prefill the form with the current values whenever the dialog opens.
  useEffect(() => {
    if (!open || !server) return;
    setName(server.name);
    setDescription(server.description ?? "");
    setUrl(server.url);
    setProtocolMode(server.protocol_mode ?? "auto");
    setElicitationPolicy(server.elicitation_policy ?? "url");
    setServiceConnection(server.service_connection_provider === "github" ? "github" : "none");
    setApiKey("");
    setFieldErrors({});
    updateServer.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, server]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!server) return;

    const parsed = mcpServerEditFormSchema.safeParse({
      name,
      description,
      url,
      protocol_mode: protocolMode,
      elicitation_policy: elicitationPolicy,
      service_connection_provider: serviceConnection,
    });
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }
    const freshApiKey = apiKey.trim();
    if (needsFreshApiKey && !freshApiKey) {
      setFieldErrors({
        api_key: "Re-enter the API key to move this server to a new origin",
      });
      return;
    }
    setFieldErrors({});

    try {
      await updateServer.mutateAsync({
        name: parsed.data.name,
        // Send the trimmed string (may be empty) so clearing the field persists;
        // the backend treats an omitted field as "no change".
        description: description.trim(),
        url: parsed.data.url,
        protocol_mode: parsed.data.protocol_mode,
        elicitation_policy: parsed.data.elicitation_policy,
        // An empty string clears it.
        service_connection_provider:
          parsed.data.service_connection_provider === "none"
            ? ""
            : parsed.data.service_connection_provider,
        ...(needsFreshApiKey ? { api_key: freshApiKey } : {}),
        ...(clearsHeaders ? { headers: {} } : {}),
      });
      onOpenChange(false);
    } catch {
      // Error is surfaced below via updateServer.error.
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit MCP Server</DialogTitle>
          <DialogDescription>
            Update the name, description, URL, protocol compatibility, and elicitation for this MCP
            server.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="edit-name">Name</Label>
            <Input
              id="edit-name"
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
            <Label htmlFor="edit-description">Description (optional)</Label>
            <Textarea
              id="edit-description"
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
            <Label htmlFor="edit-url">URL</Label>
            <Input
              id="edit-url"
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
          {needsFreshApiKey && (
            <div className="space-y-2">
              <Label htmlFor="edit-api-key">API Key</Label>
              <Input
                id="edit-api-key"
                type="password"
                value={apiKey}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                  setApiKey(e.target.value);
                  setFieldErrors((prev) => ({ ...prev, api_key: undefined }));
                }}
                aria-invalid={!!fieldErrors.api_key}
                autoComplete="off"
              />
              <p className="text-xs text-muted-foreground">
                The stored key stays with the original origin. Enter the key for the new URL.
              </p>
              {fieldErrors.api_key && (
                <p className="text-xs text-destructive">{fieldErrors.api_key}</p>
              )}
            </div>
          )}
          {clearsHeaders && (
            <p className="text-xs text-muted-foreground">
              Custom headers stay with the original origin and will be removed. Re-add them after
              saving if the new server needs them.
            </p>
          )}
          <div className="space-y-2">
            <Label htmlFor="edit-protocol-mode">Protocol compatibility</Label>
            <Select
              value={protocolMode}
              onValueChange={(value) => setProtocolMode(value as McpProtocolMode)}
            >
              <SelectTrigger id="edit-protocol-mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auto">Auto (negotiate)</SelectItem>
                <SelectItem value="2026-07-28">2026-07-28 — stateless</SelectItem>
                <SelectItem value="2025-06-18">2025-06-18 — stateful</SelectItem>
                <SelectItem value="2025-03-26">2025-03-26 — stateful</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <ElicitationPolicyField
            id="edit-elicitation-policy"
            value={elicitationPolicy}
            onChange={setElicitationPolicy}
          />
          <ServiceConnectionField
            id="edit-service-connection"
            value={serviceConnection}
            onChange={setServiceConnection}
          />
          <p className="text-xs text-muted-foreground">
            Authentication ({server?.auth_mode ?? "none"}) is managed separately. Use the Set Key
            action to update an API key.
          </p>
          {updateServer.error && (
            <p className="text-sm text-destructive">Error: {updateServer.error.message}</p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={updateServer.isPending || !name || !url}>
              {updateServer.isPending ? "Saving..." : "Save"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SetApiKeyDialog({
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

export function ManageHeadersDialog({
  server,
  open,
  onOpenChange,
}: {
  server: McpServer | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [rows, setRows] = useState<Array<{ key: string; value: string; existing: boolean }>>([]);
  const [headerErrors, setHeaderErrors] = useState<string | null>(null);
  const updateServer = useUpdateMcpServer(server?.id ?? "");

  useEffect(() => {
    if (!open || !server) return;
    const existing = Object.keys(server.headers ?? {}).map((key) => ({
      key,
      value: "",
      existing: true,
    }));
    setRows(existing.length > 0 ? existing : [{ key: "", value: "", existing: false }]);
    setHeaderErrors(null);
  }, [open, server]);

  const existingKeyCount = Object.keys(server?.headers ?? {}).length;
  const keptExistingCount = rows.filter((r) => r.existing).length;
  const isDirty = rows.some((r) => r.value.trim()) || keptExistingCount < existingKeyCount;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!server) return;

    const missingKey = rows.find((r) => !r.key.trim() && r.value.trim());
    if (missingKey) {
      setHeaderErrors("All headers must have a name.");
      return;
    }
    const activeKeys = rows.map((r) => r.key.trim()).filter(Boolean);
    if (new Set(activeKeys).size !== activeKeys.length) {
      setHeaderErrors("Header names must be unique.");
      return;
    }
    setHeaderErrors(null);

    if (!isDirty) {
      onOpenChange(false);
      return;
    }

    // Backend replaces the entire headers JSON column. Only include rows where both
    // key and value are non-empty; blank-value rows are effectively deleted.
    const headersRecord: Record<string, string> = {};
    for (const row of rows) {
      if (row.key.trim() && row.value.trim()) {
        headersRecord[row.key.trim()] = row.value.trim();
      }
    }

    await updateServer.mutateAsync({ headers: headersRecord });
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Custom Headers</DialogTitle>
          <DialogDescription>
            Header values are write-only and not shown. Enter a value to keep or update a header;
            leave it blank or remove the row to delete it. Saving replaces all headers.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-sm font-medium">Headers</span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 text-xs"
                onClick={() =>
                  setRows((prev) => [...prev, { key: "", value: "", existing: false }])
                }
              >
                <Plus className="h-3 w-3 mr-1" />
                Add header
              </Button>
            </div>
            {rows.length > 0 && (
              <div className="space-y-2">
                {rows.map((row, index) => (
                  <div key={index} className="flex items-center gap-2">
                    <Input
                      placeholder="Name"
                      value={row.key}
                      onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                        setRows((prev) =>
                          prev.map((r, i) => (i === index ? { ...r, key: e.target.value } : r)),
                        );
                        setHeaderErrors(null);
                      }}
                      className="flex-1"
                    />
                    <Input
                      placeholder={row.existing ? "stored" : "Value"}
                      value={row.value}
                      onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                        setRows((prev) =>
                          prev.map((r, i) => (i === index ? { ...r, value: e.target.value } : r)),
                        );
                        setHeaderErrors(null);
                      }}
                      className="flex-1"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      aria-label="Remove header"
                      className="h-9 w-9 p-0 shrink-0"
                      onClick={() => setRows((prev) => prev.filter((_, i) => i !== index))}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
            {headerErrors && <p className="text-xs text-destructive">{headerErrors}</p>}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={updateServer.isPending || !isDirty}>
              {updateServer.isPending ? "Saving..." : "Save Headers"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function ArchiveConfirmDialog({
  server,
  open,
  onOpenChange,
}: {
  server: McpServer | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const archiveServer = useDeleteMcpServer();
  const usage = useMcpServerUsage(open ? server?.id : undefined);

  const handleArchive = async () => {
    if (!server) return;
    await archiveServer.mutateAsync(server.id);
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
        {usage.isLoading && <p className="text-sm text-muted-foreground">Checking agent usage…</p>}
        {usage.error && (
          <p className="text-sm text-destructive">
            Agent usage could not be loaded. Archiving is blocked until the impact is known.
          </p>
        )}
        {usage.data && usage.data.total_count === 0 && (
          <p className="text-sm text-muted-foreground">No active agents use this preset.</p>
        )}
        {usage.data && usage.data.total_count > 0 && (
          <div className="space-y-2 text-sm">
            <p>
              This preset is used by {usage.data.total_count}{" "}
              {pluralize(usage.data.total_count, "active agent")}:
            </p>
            <ul className="list-disc space-y-1 pl-5">
              {usage.data.agent_names.map((name) => (
                <li key={name}>{name}</li>
              ))}
            </ul>
            {usage.data.truncated && (
              <p className="text-muted-foreground">
                Only the first {usage.data.agent_names.length} agents are shown.
              </p>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={handleArchive}
            disabled={archiveServer.isPending || usage.isLoading || !!usage.error}
          >
            {archiveServer.isPending ? "Archiving..." : "Archive"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
