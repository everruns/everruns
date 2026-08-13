"use client";

import { useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { SearchInput } from "@/components/ui/search-input";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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
import { usePageTitle } from "@/hooks";
import { Plus, Trash2, Key, X, Pencil } from "lucide-react";
import type {
  McpServer,
  CreateMcpServerRequest,
  McpServerAuthMode,
  McpProtocolMode,
} from "@/lib/api/types";
import {
  apiKeySecretSchema,
  getFieldErrors,
  mcpServerEditFormSchema,
  mcpServerFormSchema,
  type FieldErrors,
} from "@/lib/form-validation";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isArchivedStatus,
} from "@/lib/entity-lifecycle";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  IconTile,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
} from "@/components/layout";
import { registryDomainIcons } from "@/lib/registry-navigation";
import { pluralize } from "@/lib/formatting";
import { EntityIdentity } from "@/components/ui/entity-identity";

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
  canDestroy,
  onEdit,
  onDelete,
  onArchive,
  onSetApiKey,
  onManageHeaders,
}: {
  server: McpServer;
  canDestroy: boolean;
  onEdit: (server: McpServer) => void;
  onDelete: (server: McpServer) => void;
  onArchive: (server: McpServer) => void;
  onSetApiKey: (server: McpServer) => void;
  onManageHeaders: (server: McpServer) => void;
}) {
  const isArchived = server.status === "archived";
  const isDeleted = server.status === "deleted";

  return (
    <TableRow>
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
            <div className="max-w-[42ch] truncate text-xs text-muted-foreground">
              {server.description || server.url}
            </div>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant="secondary" className="font-mono">
          {server.transport_type.toUpperCase()}
        </Badge>
      </TableCell>
      <TableCell>
        <Badge variant="outline">{protocolModeLabel(server.protocol_mode)}</Badge>
      </TableCell>
      <TableCell>
        <Badge variant={getEntityStatusBadgeVariant(server.status)}>{server.status}</Badge>
      </TableCell>
      <TableCell>
        <div className="flex items-center justify-end gap-2">
          {server.auth_mode === "api_key" && (
            <Button variant="outline" size="sm" onClick={() => onSetApiKey(server)}>
              <Key className="h-4 w-4 mr-1" />
              {server.api_key_set ? "Update Key" : "Set Key"}
            </Button>
          )}
          {!isArchived && !isDeleted && (
            <Button variant="outline" size="sm" onClick={() => onEdit(server)}>
              <Pencil className="h-4 w-4 mr-1" />
              Edit
            </Button>
          )}
          {!isArchived && !isDeleted && (
            <Button variant="outline" size="sm" onClick={() => onManageHeaders(server)}>
              Headers
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
      </TableCell>
    </TableRow>
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
  const [protocolMode, setProtocolMode] = useState<McpProtocolMode>("auto");
  const [headers, setHeaders] = useState<Array<{ key: string; value: string }>>([]);
  const [headerErrors, setHeaderErrors] = useState<string | null>(null);
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
    setHeaders([]);
    setHeaderErrors(null);
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
      api_key: parsed.data.auth_mode === "api_key" ? parsed.data.api_key : undefined,
      headers: headersRecord,
    };
    await createServer.mutateAsync(data);
    onOpenChange(false);
    setName("");
    setDescription("");
    setUrl("");
    setApiKey("");
    setAuthMode("none");
    setProtocolMode("auto");
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

function EditMcpServerDialog({
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
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  const updateServer = useUpdateMcpServer(server?.id ?? "");

  // Prefill the form with the current values whenever the dialog opens.
  useEffect(() => {
    if (!open || !server) return;
    setName(server.name);
    setDescription(server.description ?? "");
    setUrl(server.url);
    setProtocolMode(server.protocol_mode ?? "auto");
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
    });
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
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
            Update the name, description, URL, and protocol compatibility for this MCP server.
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

function ManageHeadersDialog({
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

export default function McpServersPage() {
  usePageTitle("MCP Servers");
  // Always load archived so the facet rail and tab counts stay accurate; filter client-side.
  const { data: servers, isLoading, error } = useMcpServers({ includeArchived: true });
  const destroyServer = useDestroyMcpServer();
  const { can: canPolicies } = usePolicies("mcp-servers");
  const canDestroy = canPolicies("mcp_server.dangerous");

  const [search, setSearch] = useState("");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [addServerOpen, setAddServerOpen] = useState(false);
  const [editServer, setEditServer] = useState<McpServer | null>(null);
  const [apiKeyServer, setApiKeyServer] = useState<McpServer | null>(null);
  const [headersServer, setHeadersServer] = useState<McpServer | null>(null);
  const [pendingDeleteServer, setPendingDeleteServer] = useState<McpServer | null>(null);
  const [pendingArchiveServer, setPendingArchiveServer] = useState<McpServer | null>(null);

  const handleDeleteServer = async () => {
    if (!pendingDeleteServer) return;
    await destroyServer.mutateAsync(pendingDeleteServer.id);
    setPendingDeleteServer(null);
  };

  const counts = useMemo(() => {
    const list = servers ?? [];
    const archived = list.filter((s) => isArchivedStatus(s.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [servers]);

  // Transport facet counts computed from loaded data.
  const transportFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const server of servers ?? []) {
      const key = server.transport_type.toUpperCase();
      tally.set(key, (tally.get(key) ?? 0) + 1);
    }
    return [...tally.entries()].map(([transport, count]) => ({ transport, count }));
  }, [servers]);

  const filteredServers = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (servers ?? []).filter((server) => {
      if (statusTab === "active" && isArchivedStatus(server.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(server.status)) return false;
      if (!query) return true;
      const haystack = [server.name, server.description, server.url]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [servers, search, statusTab]);

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "MCP servers" }]} />

      <PageMasthead
        icon={<McpIcon />}
        title="MCP Servers"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Model Context Protocol servers that extend agents with external tools and resources."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <Button variant="accent" onClick={() => setAddServerOpen(true)}>
            <Plus className="size-4" />
            Add Server
          </Button>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search servers…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          containerClassName="w-64"
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={filteredServers}
            errorMessagePrefix="Failed to load MCP servers"
            loadingSkeleton={
              <Table>
                <TableBody>
                  {[...Array(3)].map((_, i) => (
                    <McpServerRowSkeleton key={i} />
                  ))}
                </TableBody>
              </Table>
            }
            emptyState={
              <EmptyState
                icon={<McpIcon />}
                title={
                  search || statusTab !== "active"
                    ? "No MCP servers match your filters"
                    : "No MCP servers configured"
                }
                description={
                  !search && statusTab === "active"
                    ? "Add an MCP server to extend your agents with external tools and resources."
                    : undefined
                }
                action={
                  !search &&
                  statusTab === "active" && (
                    <Button variant="accent" onClick={() => setAddServerOpen(true)}>
                      <Plus className="size-4" />
                      Add Server
                    </Button>
                  )
                }
              />
            }
          >
            {(items) => (
              <div className="border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Server</TableHead>
                      <TableHead>Transport</TableHead>
                      <TableHead>Protocol</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead className="text-right">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {items.map((server) => (
                      <McpServerRow
                        key={server.id}
                        server={server}
                        canDestroy={canDestroy}
                        onEdit={setEditServer}
                        onDelete={setPendingDeleteServer}
                        onArchive={setPendingArchiveServer}
                        onSetApiKey={setApiKeyServer}
                        onManageHeaders={setHeadersServer}
                      />
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </QueryStateWrapper>
        </PageMain>

        <PageRail>
          <RailSection label="Status">
            <div className="flex flex-col gap-1.5 text-[13px]">
              {statusItems.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => setStatusTab(item.value)}
                  className={
                    statusTab === item.value
                      ? "flex items-center justify-between text-foreground transition-colors"
                      : "flex items-center justify-between text-muted-foreground transition-colors hover:text-foreground"
                  }
                >
                  <span>{item.label}</span>
                  <span className="text-muted-foreground">{counts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>

          {transportFacets.length > 0 && (
            <RailSection label="Transport">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {transportFacets.map((facet) => (
                  <div key={facet.transport} className="flex items-center justify-between">
                    <span>{facet.transport}</span>
                    <span>{facet.count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredServers.length} of {counts.all} {pluralize(counts.all, "server")}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            View archived →
          </button>
        )}
      </PageFooter>

      {/* Dialogs */}
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
    </PageContainer>
  );
}
