"use client";

import { useId, useState, type FormEvent } from "react";
import { usePageReadContext } from "@/components/chat-mode/use-page-read-context";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, LockKeyhole, Plus, RefreshCw, ShieldCheck, Users, Trash2 } from "lucide-react";
import { useOrg } from "@/providers/org-provider";
import * as credentials from "@/lib/api/credential-bindings";
import type { CredentialOwner, ScopedCredential, HttpBinding } from "@/lib/api/credential-bindings";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { AgentSelect } from "@/components/agent/agent-select";

const DESCRIPTIONS: Record<CredentialOwner["scope"], string> = {
  virtual_user:
    "Your credentials in this organization. Agents use them only when acting as this virtual user.",
  agent: "Credentials for this agent’s authorized runs, including scheduled work.",
  session: "Credentials for this session and the selected acting identity.",
  organization:
    "Organization-owned credentials. Grant access explicitly to the agents that need them.",
};
function message(error: unknown) {
  return error instanceof Error ? error.message : "The request failed. Please try again.";
}
function expiry(input: string) {
  return input ? new Date(input).toISOString() : undefined;
}

function ValueForm({
  credential,
  onSaved,
  onCancel,
}: {
  credential: ScopedCredential;
  onSaved: () => Promise<unknown>;
  onCancel: () => void;
}) {
  usePageReadContext({
    fields: ["interaction"],
    values: { interaction: "Credential values are entered manually and are unavailable to Chat." },
    entityType: "manual_dialog",
    label: "Credential value",
    formId: "credential_value",
    priority: 100,
  });
  const [value, setValue] = useState("");
  const [expires, setExpires] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError(null);
    try {
      await credentials.saveCredentialValue(
        credential.id,
        value,
        credential.revision,
        expiry(expires),
      );
      setValue("");
      await onSaved();
      onCancel();
    } catch (error) {
      setError(message(error));
    } finally {
      setValue("");
      setPending(false);
    }
  };
  return (
    <form
      onSubmit={submit}
      autoComplete="off"
      className="mt-4 space-y-3 border-t border-border pt-4"
    >
      {credential.source_connection_id && (
        <p className="text-sm text-muted-foreground">
          Saving a manual value replaces this connection reference and stops automatic token refresh
          for this binding.
        </p>
      )}
      <div className="space-y-1.5">
        <Label htmlFor={`${id}-value`}>
          {credential.configured ? "Replacement value" : "Credential value"}
        </Label>
        <Input
          id={`${id}-value`}
          type="password"
          autoComplete="new-password"
          autoCapitalize="none"
          spellCheck={false}
          value={value}
          maxLength={65536}
          onChange={(e) => setValue(e.target.value)}
          placeholder="Paste securely"
          required
          disabled={pending}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor={`${id}-expires`}>
          Expires at <span className="font-normal text-muted-foreground">(optional)</span>
        </Label>
        <Input
          id={`${id}-expires`}
          type="datetime-local"
          value={expires}
          onChange={(e) => setExpires(e.target.value)}
          disabled={pending}
        />
      </div>
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <ShieldCheck className="size-3.5 shrink-0" />
        Encrypted on save. The value is never returned or added to chat.
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="ghost"
          onClick={() => {
            setValue("");
            onCancel();
          }}
          disabled={pending}
        >
          Cancel
        </Button>
        <Button type="submit" disabled={pending || !value}>
          {pending ? "Saving…" : "Save value"}
        </Button>
      </div>
    </form>
  );
}

function AccessDialog({
  credential,
  open,
  onOpenChange,
}: {
  credential: ScopedCredential;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { currentOrg, hasRole } = useOrg();
  const canGrantOrganization = hasRole("admin");
  const [agent, setAgent] = useState("");
  const [audience, setAudience] = useState("agent");
  const [expires, setExpires] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const grants = useQuery({
    queryKey: ["credential-grants", currentOrg?.public_id, credential.id],
    enabled: open,
    queryFn: () => credentials.listCredentialGrants(credential.id),
  });
  const save = async (event: FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError(null);
    try {
      await credentials.grantCredential(
        credential.id,
        audience === "organization" ? undefined : agent,
        expiry(expires),
      );
      await grants.refetch();
      setAgent("");
    } catch (error) {
      setError(message(error));
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        chatProtected="Credential controls"
        className="max-h-[calc(100svh-2rem)] max-w-lg overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>Access to {credential.label}</DialogTitle>
          <DialogDescription>
            Grant use of this credential without revealing its value. Access can be revoked at any
            time.
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-48 space-y-2 overflow-auto">
          {grants.isLoading ? (
            <Skeleton className="h-12" />
          ) : grants.error ? (
            <p role="alert" className="text-sm text-destructive">
              Could not load access grants.
            </p>
          ) : grants.data?.length ? (
            grants.data.map((grant) => (
              <div
                key={grant.id}
                className="flex items-center justify-between gap-3 border border-border p-3"
              >
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">
                    {grant.consumer_label ??
                      (grant.consumer_scope === "organization"
                        ? "All service agents"
                        : grant.consumer_id)}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {grant.expires_at
                      ? `Until ${new Date(grant.expires_at).toLocaleString()}`
                      : "Until revoked"}
                  </p>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={pending}
                  onClick={async () => {
                    setPending(true);
                    setError(null);
                    try {
                      await credentials.revokeCredentialGrant(credential.id, grant.id);
                      await grants.refetch();
                    } catch (error) {
                      setError(message(error));
                    } finally {
                      setPending(false);
                    }
                  }}
                >
                  Revoke access
                </Button>
              </div>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">
              {credential.scope === "organization"
                ? "No agents have access yet."
                : "No additional access granted."}
            </p>
          )}
        </div>
        <form onSubmit={save} className="space-y-3 border-t border-border pt-4">
          <div className="space-y-1.5">
            <Label htmlFor="credential-grant-audience">Grant access to</Label>
            <Select value={audience} onValueChange={setAudience}>
              <SelectTrigger id="credential-grant-audience">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="agent">One agent</SelectItem>
                {canGrantOrganization && (
                  <SelectItem value="organization">
                    All service agents in this organization
                  </SelectItem>
                )}
              </SelectContent>
            </Select>
          </div>
          {audience === "agent" && (
            <AgentSelect
              ariaLabel="Agent receiving credential access"
              value={agent}
              onValueChange={setAgent}
              placeholder="Choose an agent"
              disabled={pending}
            />
          )}
          <div className="space-y-1.5">
            <Label htmlFor="grant-expiry">
              Access expires <span className="font-normal text-muted-foreground">(optional)</span>
            </Label>
            <Input
              id="grant-expiry"
              type="datetime-local"
              value={expires}
              onChange={(e) => setExpires(e.target.value)}
              disabled={pending}
            />
          </div>
          <p className="text-xs leading-5 text-muted-foreground">
            Authorized service runs can act with this credential, including unattended runs. This
            grants use, not permission to manage or share it further. Responses may be visible to
            participants in those sessions.
          </p>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="submit" disabled={pending || (audience === "agent" && !agent)}>
              {pending ? "Saving…" : "Grant access"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function AddBinding({
  owner,
  open,
  onOpenChange,
  onSaved,
}: {
  owner: CredentialOwner;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => Promise<unknown>;
}) {
  const [kind, setKind] = useState("http");
  const [name, setName] = useState("");
  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("");
  const [auth, setAuth] = useState("bearer");
  const [field, setField] = useState("");
  const [server, setServer] = useState("");
  const [tool, setTool] = useState("");
  const [parameter, setParameter] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError(null);
    try {
      if (kind === "mcp_parameter") {
        await credentials.createScopedCredential(owner, {
          name: parameter,
          label,
          mcp: { server_name: server, url, tool_name: tool, parameter_name: parameter },
        });
      } else {
        const binding: HttpBinding = {
          url,
          auth:
            auth === "bearer"
              ? { type: "bearer" }
              : { type: auth as "header" | "form_field", name: field },
        };
        await credentials.createScopedCredential(owner, { name, label, binding });
      }
      await onSaved();
      onOpenChange(false);
    } catch (error) {
      setError(message(error));
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        chatProtected="Credential controls"
        className="max-h-[calc(100svh-2rem)] max-w-lg overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>Add credential binding</DialogTitle>
          <DialogDescription>
            Choose where the credential may be used. You will add its value securely after reviewing
            the binding.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          {
            <div className="space-y-1.5">
              <Label htmlFor={`${id}-kind`}>Binding type</Label>
              <Select value={kind} onValueChange={setKind}>
                <SelectTrigger id={`${id}-kind`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="http">HTTP service</SelectItem>
                  <SelectItem value="mcp_parameter">MCP tool parameter</SelectItem>
                </SelectContent>
              </Select>
            </div>
          }
          <div className="space-y-1.5">
            <Label htmlFor={`${id}-label`}>Label</Label>
            <Input
              id={`${id}-label`}
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="Shipmail account"
              maxLength={128}
              required
              disabled={pending}
            />
          </div>
          {kind === "http" ? (
            <>
              <div className="space-y-1.5">
                <Label htmlFor={`${id}-name`}>Reference name</Label>
                <Input
                  id={`${id}-name`}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="SHIPMAIL_TOKEN"
                  pattern="[A-Za-z_][A-Za-z0-9_]{0,127}"
                  required
                  disabled={pending}
                />
                <p className="text-xs text-muted-foreground">
                  The agent uses ${name || "NAME"}. It never reads the value.
                </p>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor={`${id}-url`}>Allowed HTTPS URL</Label>
                <Input
                  id={`${id}-url`}
                  type="url"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://api.example.com/v1"
                  pattern="https://.*"
                  required
                  disabled={pending}
                />
                <p className="text-xs text-muted-foreground">
                  A host and optional path prefix. This does not grant network access.
                </p>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor={`${id}-auth`}>Send credential as</Label>
                <Select value={auth} onValueChange={setAuth}>
                  <SelectTrigger id={`${id}-auth`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="bearer">Bearer authorization</SelectItem>
                    <SelectItem value="header">Custom HTTP header</SelectItem>
                    <SelectItem value="form_field">Named form field</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {auth !== "bearer" && (
                <div className="space-y-1.5">
                  <Label htmlFor={`${id}-field`}>
                    {auth === "header" ? "Header name" : "Form field name"}
                  </Label>
                  <Input
                    id={`${id}-field`}
                    value={field}
                    onChange={(e) => setField(e.target.value)}
                    placeholder={auth === "header" ? "X-API-Key" : "assertion"}
                    required
                    disabled={pending}
                  />
                </div>
              )}
            </>
          ) : (
            <>
              <div className="space-y-1.5">
                <Label htmlFor={`${id}-mcp-url`}>Exact MCP HTTPS URL</Label>
                <Input
                  id={`${id}-mcp-url`}
                  type="url"
                  pattern="https://.*"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  required
                  disabled={pending}
                  placeholder="https://mcp.example.com/mcp"
                />
              </div>

              <div className="space-y-1.5">
                <Label htmlFor={`${id}-server`}>MCP server name</Label>
                <Input
                  id={`${id}-server`}
                  value={server}
                  onChange={(e) => setServer(e.target.value)}
                  required
                  disabled={pending}
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1.5">
                  <Label htmlFor={`${id}-tool`}>Tool</Label>
                  <Input
                    id={`${id}-tool`}
                    value={tool}
                    onChange={(e) => setTool(e.target.value)}
                    required
                    disabled={pending}
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor={`${id}-parameter`}>Parameter</Label>
                  <Input
                    id={`${id}-parameter`}
                    value={parameter}
                    onChange={(e) => setParameter(e.target.value)}
                    required
                    disabled={pending}
                  />
                </div>
              </div>
            </>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={pending}>
              {pending ? "Adding…" : "Add binding"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ScopedCredentialsPanelContent({
  owner,
  title = "Secure credentials",
}: {
  owner: CredentialOwner;
  title?: string;
}) {
  const { currentOrg } = useOrg();
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [sharing, setSharing] = useState<ScopedCredential | null>(null);
  const [revoking, setRevoking] = useState<ScopedCredential | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const queryKey = [
    "scoped-credentials",
    currentOrg?.public_id,
    owner.scope,
    owner.owner_id,
    owner.usage,
  ];
  const query = useQuery({
    queryKey,
    enabled: !!currentOrg,
    queryFn: () => credentials.listScopedCredentials(owner),
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey });
  const revoke = async () => {
    if (!revoking) return;
    setPending(true);
    setError(null);
    try {
      await credentials.revokeScopedCredential(revoking.id, revoking.revision);
      await refresh();
      setRevoking(null);
    } catch (error) {
      setError(message(error));
    } finally {
      setPending(false);
    }
  };
  return (
    <section className="border border-border bg-card" aria-label={title}>
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border p-4">
        <div>
          <h3 className="flex items-center gap-2 font-medium">
            <LockKeyhole className="size-4" />
            {title}
          </h3>
          <p className="mt-1 max-w-xl text-sm leading-5 text-muted-foreground">
            {owner.scope === "virtual_user" && owner.usage === "service"
              ? "Credentials for this service virtual user’s authorized runs."
              : DESCRIPTIONS[owner.scope]}
          </p>
        </div>
        <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
          <Plus className="size-4" />
          Add binding
        </Button>
      </div>
      <div className="space-y-3 p-4">
        {query.isLoading ? (
          <Skeleton className="h-28" />
        ) : query.error ? (
          <div role="alert" className="space-y-2 text-sm text-destructive">
            <p>Could not load credentials. {message(query.error)}</p>
            <Button size="sm" variant="outline" onClick={() => void query.refetch()}>
              Retry
            </Button>
          </div>
        ) : query.data?.length ? (
          query.data.map((credential) => {
            const expired =
              !!credential.expires_at && new Date(credential.expires_at) <= new Date();
            return (
              <article
                key={`${currentOrg?.public_id}:${credential.id}`}
                className="border border-border p-4"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <KeyRound className="size-4 shrink-0 text-muted-foreground" />
                      <h4 className="font-medium">{credential.label}</h4>
                      <Badge variant={expired ? "destructive" : "outline"}>
                        {expired
                          ? "Expired"
                          : credential.configured
                            ? "Configured"
                            : "Setup required"}
                      </Badge>
                    </div>
                    {credential.source_connection_id && (
                      <p className="mt-1 text-xs text-muted-foreground">
                        Uses the selected GitHub App connection; tokens refresh automatically.
                      </p>
                    )}
                    {credential.name && (
                      <code className="mt-1 block text-xs text-muted-foreground">
                        ${credential.name}
                      </code>
                    )}
                    <p className="mt-2 break-all text-sm">
                      {credential.binding?.url ?? credential.mcp_server_url}
                    </p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {credential.binding
                        ? credential.binding.auth.type === "bearer"
                          ? "Authorization: Bearer"
                          : "name" in credential.binding.auth
                            ? `${credential.binding.auth.type === "header" ? "Header" : "Form field"}: ${credential.binding.auth.name}`
                            : ""
                        : `${credential.tool_name} → ${credential.parameter_name}`}
                    </p>
                    {credential.expires_at && (
                      <p className="mt-1 text-xs text-muted-foreground">
                        Expires {new Date(credential.expires_at).toLocaleString()}
                      </p>
                    )}
                    {credential.last_used_at && (
                      <p className="mt-1 text-xs text-muted-foreground">
                        Last used {new Date(credential.last_used_at).toLocaleString()}
                        {credential.last_used_agent_id
                          ? ` · ${credential.last_used_agent_name ?? "Service agent"}`
                          : ""}
                      </p>
                    )}
                  </div>
                  <div className="flex flex-wrap items-center gap-1">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => setEditing(editing === credential.id ? null : credential.id)}
                    >
                      {credential.configured ? (
                        <RefreshCw className="size-3.5" />
                      ) : (
                        <KeyRound className="size-3.5" />
                      )}
                      {credential.source_connection_id
                        ? "Use a manual value"
                        : credential.configured
                          ? "Replace"
                          : "Add value"}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setSharing(credential)}>
                      <Users className="size-3.5" />
                      Access
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-label={`Revoke ${credential.label}`}
                      onClick={() => setRevoking(credential)}
                    >
                      <Trash2 className="size-4 text-destructive" />
                    </Button>
                  </div>
                </div>
                {editing === credential.id && (
                  <ValueForm
                    key={`${currentOrg?.public_id}:${credential.id}:${credential.revision}`}
                    credential={credential}
                    onSaved={refresh}
                    onCancel={() => setEditing(null)}
                  />
                )}
              </article>
            );
          })
        ) : (
          <div className="flex flex-col items-center justify-center border border-dashed border-border px-5 py-9 text-center">
            <LockKeyhole className="mb-3 size-7 text-muted-foreground" />
            <p className="text-sm font-medium">No credentials yet</p>
            <p className="mt-1 max-w-sm text-sm text-muted-foreground">
              Add a binding to connect a service securely. Values stay encrypted and out of chat.
            </p>
          </div>
        )}
      </div>
      {adding && (
        <AddBinding
          key={`${currentOrg?.public_id}:${owner.scope}:${owner.owner_id}`}
          owner={owner}
          open={adding}
          onOpenChange={setAdding}
          onSaved={refresh}
        />
      )}
      {sharing && (
        <AccessDialog
          key={`${currentOrg?.public_id}:${sharing.id}`}
          credential={sharing}
          open
          onOpenChange={(open) => {
            if (!open) setSharing(null);
          }}
        />
      )}
      <Dialog
        open={!!revoking}
        onOpenChange={(open) => {
          if (!open) {
            setRevoking(null);
            setError(null);
          }
        }}
      >
        <DialogContent chatProtected="Credential controls" className="max-w-md">
          <DialogHeader>
            <DialogTitle>Revoke {revoking?.label}?</DialogTitle>
            <DialogDescription>
              The next request will be blocked. This also removes access grants. Requests already
              sent cannot be recalled.
            </DialogDescription>
          </DialogHeader>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRevoking(null)} disabled={pending}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void revoke()} disabled={pending}>
              {pending ? "Revoking…" : "Revoke credential"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

export function ScopedCredentialsPanel(props: { owner: CredentialOwner; title?: string }) {
  const { currentOrg } = useOrg();
  return (
    <ScopedCredentialsPanelContent
      key={`${currentOrg?.public_id}:${props.owner.scope}:${props.owner.owner_id}:${props.owner.usage}`}
      {...props}
    />
  );
}
