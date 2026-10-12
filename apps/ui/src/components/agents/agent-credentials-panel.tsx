"use client";

import { FormEvent, useState } from "react";
import { KeyRound, LockKeyhole, Plus, RefreshCw, Trash2 } from "lucide-react";
import type { AgentCredentialBinding } from "@/lib/api/types";
import type { CreateAgentCredentialBindingRequest } from "@/lib/api/agents";
import {
  useAgentCredentials,
  useCreateAgentCredentialBinding,
  useDeleteAgentCredentialBinding,
  useSetAgentCredentialValue,
} from "@/hooks/use-agents";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";

function SecretValueForm({
  agentId,
  bindingId,
  configured,
}: {
  agentId: string;
  bindingId: string;
  configured: boolean;
}) {
  const save = useSetAgentCredentialValue(agentId);
  const [value, setValue] = useState("");
  const [editing, setEditing] = useState(!configured);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!value) return;
    try {
      await save.mutateAsync({ bindingId, value });
      setEditing(false);
    } finally {
      setValue("");
    }
  };

  if (!editing) {
    return (
      <Button variant="outline" size="sm" onClick={() => setEditing(true)}>
        <RefreshCw className="size-4" /> Rotate
      </Button>
    );
  }

  return (
    <form onSubmit={submit} className="flex min-w-64 items-end gap-2" autoComplete="off">
      <div className="flex-1 space-y-1.5">
        <Label htmlFor={`credential-${bindingId}`}>
          {configured ? "Replacement value" : "Value"}
        </Label>
        <Input
          id={`credential-${bindingId}`}
          name={`credential-${bindingId}`}
          type="password"
          value={value}
          onChange={(event) => setValue(event.target.value)}
          autoComplete="new-password"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          maxLength={64 * 1024}
          placeholder="Enter securely"
          required
        />
      </div>
      <Button type="submit" size="sm" disabled={save.isPending || !value}>
        {save.isPending ? "Saving…" : configured ? "Replace" : "Save"}
      </Button>
      {configured && (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => {
            setValue("");
            setEditing(false);
          }}
        >
          Cancel
        </Button>
      )}
    </form>
  );
}

type CredentialKind = "http" | "mcp_parameter";

const EMPTY_DRAFT = {
  kind: "http" as CredentialKind,
  mcp_server_name: "",
  tool_name: "",
  parameter_name: "",
  env_name: "",
  url: "",
  auth: "bearer" as "bearer" | "header",
  header_name: "",
  label: "",
  description: "",
  value: "",
};

/// Where a credential goes, in one line: the MCP tool argument it fills, or
/// the env var the shell uses and the URL and header it is sent with.
function bindingTarget(binding: AgentCredentialBinding): string {
  if (binding.kind === "http") {
    const auth = binding.binding?.auth;
    const header = auth?.type === "header" ? `${auth.name} header` : "Authorization: Bearer";
    return `$${binding.env_name} → ${binding.binding?.url} (${header})`;
  }
  return `${binding.mcp_server_name} / ${binding.tool_name} → ${binding.parameter_name}`;
}

function draftRequest(draft: typeof EMPTY_DRAFT): CreateAgentCredentialBindingRequest {
  const common = {
    label: draft.label.trim(),
    description: draft.description.trim() || undefined,
  };
  if (draft.kind === "http") {
    return {
      ...common,
      env_name: draft.env_name.trim(),
      url: draft.url.trim(),
      auth: draft.auth,
      header_name: draft.auth === "header" ? draft.header_name.trim() : undefined,
    };
  }
  return {
    ...common,
    mcp_server_name: draft.mcp_server_name.trim(),
    tool_name: draft.tool_name.trim(),
    parameter_name: draft.parameter_name.trim(),
  };
}

export function AgentCredentialsPanel({ agentId }: { agentId: string }) {
  const credentials = useAgentCredentials(agentId);
  const create = useCreateAgentCredentialBinding(agentId);
  const remove = useDeleteAgentCredentialBinding(agentId);
  const setValue = useSetAgentCredentialValue(agentId);
  const [showCreate, setShowCreate] = useState(false);
  const [draft, setDraft] = useState(EMPTY_DRAFT);

  const submitNew = async (event: FormEvent) => {
    event.preventDefault();
    const secretValue = draft.value;
    try {
      const binding = await create.mutateAsync(draftRequest(draft));
      await setValue.mutateAsync({ bindingId: binding.id, value: secretValue });
      setShowCreate(false);
      setDraft(EMPTY_DRAFT);
    } finally {
      setDraft((current) => ({ ...current, value: "" }));
    }
  };

  const deleteBinding = async (id: string, label: string) => {
    if (confirm(`Revoke ${label}? Scheduled and future Agent runs will no longer receive it.`)) {
      await remove.mutateAsync(id);
    }
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div>
            <CardTitle className="flex items-center gap-2">
              <LockKeyhole className="size-5" /> Secure credentials
            </CardTitle>
            <CardDescription>
              Agent-scoped and durable across shared or per-invocation sessions. Values are
              encrypted and never readable after saving. An MCP tool credential is injected into the
              bound tool call; an HTTP API credential is a placeholder in the agent&apos;s shell,
              replaced only in requests to its URL. Neither enters prompts, chat history, or
              anything the model sees.
            </CardDescription>
          </div>
          <Button variant="outline" onClick={() => setShowCreate((value) => !value)}>
            <Plus className="size-4" /> Add binding
          </Button>
        </CardHeader>
        <CardContent className="space-y-4">
          {showCreate && (
            <form
              onSubmit={submitNew}
              className="grid gap-4 border bg-muted/30 p-4 md:grid-cols-2"
              autoComplete="off"
            >
              <div
                className="flex gap-2 md:col-span-2"
                role="radiogroup"
                aria-label="Credential type"
              >
                {(
                  [
                    ["http", "HTTP API"],
                    ["mcp_parameter", "MCP tool parameter"],
                  ] as const
                ).map(([kind, label]) => (
                  <Button
                    key={kind}
                    type="button"
                    role="radio"
                    aria-checked={draft.kind === kind}
                    variant={draft.kind === kind ? "secondary" : "ghost"}
                    size="sm"
                    onClick={() => setDraft({ ...draft, kind })}
                  >
                    {label}
                  </Button>
                ))}
              </div>
              {draft.kind === "http" ? (
                <>
                  <div className="space-y-2">
                    <Label htmlFor="credential-env">Shell variable</Label>
                    <Input
                      id="credential-env"
                      value={draft.env_name}
                      onChange={(e) => setDraft({ ...draft, env_name: e.target.value })}
                      placeholder="JEV_API_KEY"
                      pattern="[A-Za-z_][A-Za-z0-9_]*"
                      required
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="credential-url">Sent only to (HTTPS URL prefix)</Label>
                    <Input
                      id="credential-url"
                      type="url"
                      value={draft.url}
                      onChange={(e) => setDraft({ ...draft, url: e.target.value })}
                      placeholder="https://api.example.com"
                      required
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="credential-auth">Sent as</Label>
                    <div id="credential-auth" className="flex gap-2">
                      {(
                        [
                          ["bearer", "Bearer token"],
                          ["header", "Custom header"],
                        ] as const
                      ).map(([auth, label]) => (
                        <Button
                          key={auth}
                          type="button"
                          aria-pressed={draft.auth === auth}
                          variant={draft.auth === auth ? "secondary" : "ghost"}
                          size="sm"
                          onClick={() => setDraft({ ...draft, auth })}
                        >
                          {label}
                        </Button>
                      ))}
                    </div>
                  </div>
                  {draft.auth === "header" && (
                    <div className="space-y-2">
                      <Label htmlFor="credential-header">Header name</Label>
                      <Input
                        id="credential-header"
                        value={draft.header_name}
                        onChange={(e) => setDraft({ ...draft, header_name: e.target.value })}
                        placeholder="X-Api-Key"
                        required
                      />
                    </div>
                  )}
                </>
              ) : (
                <>
                  <div className="space-y-2">
                    <Label htmlFor="credential-server">Attached MCP server name</Label>
                    <Input
                      id="credential-server"
                      value={draft.mcp_server_name}
                      onChange={(e) => setDraft({ ...draft, mcp_server_name: e.target.value })}
                      required
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="credential-tool">MCP tool name</Label>
                    <Input
                      id="credential-tool"
                      value={draft.tool_name}
                      onChange={(e) => setDraft({ ...draft, tool_name: e.target.value })}
                      placeholder="visti_send"
                      required
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="credential-parameter">Secret parameter</Label>
                    <Input
                      id="credential-parameter"
                      value={draft.parameter_name}
                      onChange={(e) => setDraft({ ...draft, parameter_name: e.target.value })}
                      placeholder="channel_key"
                      required
                    />
                  </div>
                </>
              )}
              <div className="space-y-2">
                <Label htmlFor="credential-label">Label</Label>
                <Input
                  id="credential-label"
                  value={draft.label}
                  onChange={(e) => setDraft({ ...draft, label: e.target.value })}
                  placeholder={draft.kind === "http" ? "Jev API key" : "Visti channel key"}
                  required
                />
              </div>
              <div className="space-y-2 md:col-span-2">
                <Label htmlFor="credential-description">Purpose (optional)</Label>
                <Input
                  id="credential-description"
                  value={draft.description}
                  onChange={(e) => setDraft({ ...draft, description: e.target.value })}
                />
              </div>
              <div className="space-y-2 md:col-span-2">
                <Label htmlFor="credential-new-value">Value</Label>
                <Input
                  id="credential-new-value"
                  name="agent-credential-new-value"
                  type="password"
                  value={draft.value}
                  onChange={(e) => setDraft({ ...draft, value: e.target.value })}
                  autoComplete="new-password"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  maxLength={64 * 1024}
                  required
                />
              </div>
              <div className="flex gap-2 md:col-span-2">
                <Button type="submit" disabled={create.isPending || setValue.isPending}>
                  Save binding
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => {
                    setDraft({ ...draft, value: "" });
                    setShowCreate(false);
                  }}
                >
                  Cancel
                </Button>
              </div>
            </form>
          )}

          {credentials.isLoading ? (
            <Skeleton className="h-24 w-full" />
          ) : credentials.error ? (
            <p className="text-sm text-destructive">Unable to load secure credential metadata.</p>
          ) : credentials.data?.length ? (
            <div className="space-y-3">
              {credentials.data.map((binding) => (
                <div
                  key={binding.id}
                  className="flex flex-col gap-4 border p-4 lg:flex-row lg:items-center lg:justify-between"
                >
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <KeyRound className="size-4" />
                      <span className="font-medium">{binding.label}</span>
                      <Badge variant={binding.configured ? "secondary" : "destructive"}>
                        {binding.configured ? "Configured" : "Setup required"}
                      </Badge>
                    </div>
                    <p className="font-mono text-xs text-muted-foreground">
                      {bindingTarget(binding)}
                    </p>
                    {binding.description && (
                      <p className="text-sm text-muted-foreground">{binding.description}</p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      Value hidden · updated {new Date(binding.updated_at).toLocaleString()}
                    </p>
                  </div>
                  <div className="flex items-end gap-2">
                    <SecretValueForm
                      agentId={agentId}
                      bindingId={binding.id}
                      configured={binding.configured}
                    />
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Revoke ${binding.label}`}
                      className="text-destructive"
                      onClick={() => deleteBinding(binding.id, binding.label)}
                      disabled={remove.isPending}
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="border border-dashed p-8 text-center">
              <LockKeyhole className="mx-auto mb-2 size-8 text-muted-foreground" />
              <p className="font-medium">No Agent credentials</p>
              <p className="mt-1 text-sm text-muted-foreground">
                When Platform Chat attaches a credentialed MCP tool or HTTP API, its setup request
                appears here. You can also add an HTTP API key or a tool-parameter binding manually.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
