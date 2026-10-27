"use client";

// The form that connects (or rotates) an organization connection: one named
// account of a connector, with the connector's own credential fields. Sandbox
// provider accounts and web search accounts share it.

import { useEffect, useState } from "react";
import { usePageFormPrefill } from "@/components/chat-mode/use-page-form-prefill";
import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";
import { useSaveOrganizationConnection } from "@/hooks/use-organization-connections";
import type { OrganizationConnection } from "@/lib/api/organization-connections";
import type { ConnectionProvider } from "@/lib/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function OrganizationAccountDialog({
  provider,
  account,
  open,
  onOpenChange,
  onSaved,
  description = "Organization admins can assign this account to Sandbox Templates. Secrets stay encrypted and are never shown to agents.",
}: {
  provider: ConnectionProvider | null;
  account?: OrganizationConnection;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: (message: string) => void;
  /** What the account is for, shown under the title. */
  description?: string;
}) {
  const [name, setName] = useState(account?.name ?? "");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const save = useSaveOrganizationConnection();
  const chat = useTextPageFormDraft({
    fields: ["name"],
    values: { name },
    entityType: "organization_account",
    entityId: account?.id,
    label: "Account name",
    priority: 30,
    enabled: open && !!provider?.form_schema,
    baseline: JSON.stringify([account?.id, account?.name, account?.updated_at]),
    dirty: name !== (account?.name ?? ""),
    writable: !save.isPending,
    apply: (values) => setName(values.name),
  });

  usePageFormPrefill({
    identity: `${provider?.provider_id}:${account?.id ?? "new"}`,
    open,
    values: name,
    saved: account?.name ?? "",
    apply: setName,
  });

  useEffect(() => {
    if (!open) return;
    setName(account?.name ?? "");
    setFields({});
    setError(null);
  }, [account?.id, open, provider?.provider_id]);

  useEffect(() => {
    if (open && !chat.adopted) {
      setName(account?.name ?? "");
      setFields({});
      setError(null);
    }
  }, [account, provider, open, chat.adopted]);

  if (!provider?.form_schema) return null;
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (chat.blocked) return;
    const apiKey = fields.api_key ?? "";
    const extraFields = Object.fromEntries(
      Object.entries(fields).filter(([key, value]) => key !== "api_key" && value.trim()),
    );
    try {
      await save.mutateAsync({
        provider: provider.provider_id,
        id: account?.id,
        input: { name, apiKey, extraFields },
      });
      onSaved(account ? "Account updated" : "Account connected");
      onOpenChange(false);
    } catch {
      setError("Could not save the account. Check the credential and try again.");
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {account ? "Rotate" : "Connect"} {provider.display_name}
          </DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <form className="space-y-4" onSubmit={submit}>
          {chat.notice}
          {chat.adopted && chat.blocked && (
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setName(account?.name ?? "");
                chat.clear();
              }}
            >
              Discard name changes
            </Button>
          )}
          <div className="space-y-2">
            <Label htmlFor="account-name">Account name</Label>
            <Input
              id="account-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required
              placeholder="Production"
            />
          </div>
          {provider.form_schema.fields.map((field) => (
            <div className="space-y-2" key={field.name}>
              <Label htmlFor={`account-${field.name}`}>{field.label}</Label>
              <Input
                id={`account-${field.name}`}
                type={field.field_type}
                value={fields[field.name] ?? ""}
                onChange={(event) =>
                  setFields((current) => ({ ...current, [field.name]: event.target.value }))
                }
                required={
                  field.required &&
                  !(chat.adopted && account && !Object.values(fields).some((value) => value.trim()))
                }
                placeholder={field.placeholder}
                autoComplete="off"
              />
              {field.help_text ? (
                <p className="text-xs text-muted-foreground">{field.help_text}</p>
              ) : null}
            </div>
          ))}
          {error ? <p className="text-sm text-destructive">{error}</p> : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending || chat.blocked}>
              {save.isPending ? "Validating…" : "Save account"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
