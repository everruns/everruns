"use client";

import { ProviderAccountIcon } from "@/components/icons/facet-icons";
import { useEffect, useMemo, useState } from "react";
import { Plus, RefreshCw, Trash2 } from "lucide-react";
import { useConnectionProviders } from "@/hooks/use-user-connections";
import {
  useDeleteOrganizationConnection,
  useOrganizationConnections,
  useSaveOrganizationConnection,
  useVerifyOrganizationConnection,
} from "@/hooks/use-organization-connections";
import type { OrganizationConnection } from "@/lib/api/organization-connections";
import type { ConnectionProvider } from "@/lib/api/types";
import { ProviderIcon } from "@/components/connections/provider-icon";
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
import { EntityCard, EntityCardDescription, EntityCardFooter } from "@/components/ui/entity-card";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import {
  EmptyState,
  PageBreadcrumb,
  PageContainer,
  PageMain,
  PageMasthead,
} from "@/components/layout";

function AccountDialog({
  provider,
  account,
  open,
  onOpenChange,
  onSaved,
}: {
  provider: ConnectionProvider | null;
  account?: OrganizationConnection;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: (message: string) => void;
}) {
  const [name, setName] = useState(account?.name ?? "");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const save = useSaveOrganizationConnection();

  useEffect(() => {
    if (!open) return;
    setName(account?.name ?? "");
    setFields({});
    setError(null);
  }, [account, open, provider]);

  if (!provider?.form_schema) return null;
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
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
      onSaved(account ? "Provider account updated" : "Provider account connected");
      onOpenChange(false);
    } catch {
      setError("Could not save provider account. Check the credential and try again.");
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {account ? "Rotate" : "Connect"} {provider.display_name}
          </DialogTitle>
          <DialogDescription>
            Organization admins can assign this account to Sandbox Templates. Secrets stay encrypted
            and are never shown to agents.
          </DialogDescription>
        </DialogHeader>
        <form className="space-y-4" onSubmit={submit}>
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
                required={field.required}
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
            <Button type="submit" disabled={save.isPending}>
              {save.isPending ? "Validating…" : "Save account"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export default function SandboxProviderAccountsPage() {
  const connections = useOrganizationConnections();
  const { data: allProviders = [] } = useConnectionProviders();
  const providers = useMemo(
    () => allProviders.filter((provider) => provider.capabilities.includes("sandbox_provisioning")),
    [allProviders],
  );
  const [editing, setEditing] = useState<{
    provider: ConnectionProvider;
    account?: OrganizationConnection;
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const remove = useDeleteOrganizationConnection();
  const verify = useVerifyOrganizationConnection();
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Provider Accounts" }]} />
      <PageMasthead
        icon={<ProviderAccountIcon />}
        title="Sandbox Provider Accounts"
        description="Organization-owned credentials that managed Sandbox Templates can use for every Session."
      />
      <PageMain className="space-y-8">
        {notice ? <p className="border bg-muted px-4 py-3 text-sm">{notice}</p> : null}
        <QueryStateWrapper
          data={connections.data}
          isLoading={connections.isLoading}
          error={connections.error}
          emptyState={
            <EmptyState icon={<ProviderAccountIcon />} title="No organization provider accounts" />
          }
        >
          {(accounts) => (
            <div className="grid gap-4 xl:grid-cols-2">
              {accounts.map((account) => {
                const provider = providers.find(
                  (candidate) => candidate.provider_id === account.provider,
                );
                return (
                  <EntityCard
                    key={account.id}
                    icon={<ProviderIcon iconName={provider?.icon ?? "cloud"} className="size-5" />}
                    title={account.name}
                    subtitle={provider?.display_name ?? account.provider}
                    headerActions={
                      <div className="flex gap-1">
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={verify.isPending}
                          onClick={async () => {
                            try {
                              const result = await verify.mutateAsync(account.id);
                              setNotice(
                                result.valid
                                  ? "Connection verified"
                                  : (result.error ?? "Connection failed"),
                              );
                            } catch {
                              setNotice("Could not verify the provider account");
                            }
                          }}
                        >
                          <RefreshCw className="size-3.5" /> Test
                        </Button>
                        {provider ? (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => setEditing({ provider, account })}
                          >
                            Rotate
                          </Button>
                        ) : null}
                        <Button
                          size="icon"
                          variant="ghost"
                          disabled={remove.isPending}
                          aria-label={`Disconnect ${account.name}`}
                          onClick={async () => {
                            if (
                              !window.confirm(
                                `Disconnect ${account.name}? Templates using this account must be changed first.`,
                              )
                            )
                              return;
                            try {
                              await remove.mutateAsync(account.id);
                              setNotice("Provider account disconnected");
                            } catch {
                              setNotice("Account is still assigned to a sandbox or template");
                            }
                          }}
                        >
                          <Trash2 className="size-4" />
                        </Button>
                      </div>
                    }
                    footer={
                      <EntityCardFooter
                        meta={`Connected ${new Date(account.connected_at).toLocaleDateString()}`}
                      />
                    }
                  >
                    <EntityCardDescription>
                      Available to Sandbox Templates that explicitly select this account.
                    </EntityCardDescription>
                  </EntityCard>
                );
              })}
            </div>
          )}
        </QueryStateWrapper>

        <section className="space-y-3">
          <h2 className="text-lg font-semibold">Available providers</h2>
          <div className="grid gap-3 xl:grid-cols-2">
            {providers.map((provider) => (
              <div
                key={provider.provider_id}
                className="flex items-center justify-between border bg-card p-4"
              >
                <div className="flex items-center gap-3">
                  <ProviderIcon iconName={provider.icon} className="size-5" />
                  <div>
                    <p className="font-medium">{provider.display_name}</p>
                    <p className="text-sm text-muted-foreground">{provider.description}</p>
                  </div>
                </div>
                <Button variant="outline" onClick={() => setEditing({ provider })}>
                  <Plus className="size-4" /> Connect
                </Button>
              </div>
            ))}
          </div>
        </section>
      </PageMain>
      <AccountDialog
        provider={editing?.provider ?? null}
        account={editing?.account}
        open={!!editing}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        onSaved={setNotice}
      />
    </PageContainer>
  );
}
