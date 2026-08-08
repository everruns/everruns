"use client";

import { FormEvent, useState } from "react";
import { KeyRound, LockKeyhole, Plus, RefreshCw, Trash2 } from "lucide-react";
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

export function AgentCredentialsPanel({ agentId }: { agentId: string }) {
  const credentials = useAgentCredentials(agentId);
  const create = useCreateAgentCredentialBinding(agentId);
  const remove = useDeleteAgentCredentialBinding(agentId);
  const setValue = useSetAgentCredentialValue(agentId);
  const [showCreate, setShowCreate] = useState(false);
  const [draft, setDraft] = useState({
    mcp_server_name: "",
    tool_name: "",
    parameter_name: "",
    label: "",
    description: "",
    value: "",
  });

  const submitNew = async (event: FormEvent) => {
    event.preventDefault();
    const secretValue = draft.value;
    try {
      const binding = await create.mutateAsync({
        mcp_server_name: draft.mcp_server_name.trim(),
        tool_name: draft.tool_name.trim(),
        parameter_name: draft.parameter_name.trim(),
        label: draft.label.trim(),
        description: draft.description.trim() || undefined,
      });
      await setValue.mutateAsync({ bindingId: binding.id, value: secretValue });
      setShowCreate(false);
      setDraft({
        mcp_server_name: "",
        tool_name: "",
        parameter_name: "",
        label: "",
        description: "",
        value: "",
      });
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
              encrypted, never readable after saving, and injected directly into the bound MCP tool
              call without entering prompts, chat history, or model-visible tool arguments.
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
              <div className="space-y-2">
                <Label htmlFor="credential-label">Label</Label>
                <Input
                  id="credential-label"
                  value={draft.label}
                  onChange={(e) => setDraft({ ...draft, label: e.target.value })}
                  placeholder="Visti channel key"
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
                      {binding.mcp_server_name} / {binding.tool_name} → {binding.parameter_name}
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
                      size="icon"
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
                When Platform Chat attaches a credentialed MCP tool, its setup request appears here.
                You can also add an exact tool-parameter binding manually.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
