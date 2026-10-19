"use client";

import { ProviderAccountIcon } from "@/components/icons/facet-icons";
import { useMemo, useState } from "react";
import { Plus, RefreshCw, Trash2 } from "lucide-react";
import { usePageTitle } from "@/hooks";
import { useConnectionProviders } from "@/hooks/use-user-connections";
import {
  useDeleteOrganizationConnection,
  useOrganizationConnections,
  useVerifyOrganizationConnection,
} from "@/hooks/use-organization-connections";
import type { OrganizationConnection } from "@/lib/api/organization-connections";
import type { ConnectionProvider } from "@/lib/api/types";
import { ProviderIcon } from "@/components/connections/provider-icon";
import { OrganizationAccountDialog } from "@/components/connections/organization-account-dialog";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardDescription, EntityCardFooter } from "@/components/ui/entity-card";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { EmptyState, PageMain } from "@/components/layout";

// Provider accounts tab. The page header and tabs come from the shell
// (../sandboxes-shell.tsx).
export default function SandboxProviderAccountsPage() {
  usePageTitle("Provider accounts", "Sandboxes");
  const connections = useOrganizationConnections();
  const { data: allProviders = [] } = useConnectionProviders();
  const providers = useMemo(
    () => allProviders.filter((provider) => provider.capabilities.includes("sandbox_provisioning")),
    [allProviders],
  );
  // Search accounts are organization connections too; they live on the Web
  // search page, not here.
  const sandboxAccounts = useMemo(
    () =>
      connections.data?.filter(
        (account) =>
          !allProviders
            .find((provider) => provider.provider_id === account.provider)
            ?.capabilities.includes("web_search"),
      ),
    [connections.data, allProviders],
  );
  const [editing, setEditing] = useState<{
    provider: ConnectionProvider;
    account?: OrganizationConnection;
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const remove = useDeleteOrganizationConnection();
  const verify = useVerifyOrganizationConnection();
  return (
    <>
      <PageMain className="space-y-8">
        {notice ? <p className="border bg-muted px-4 py-3 text-sm">{notice}</p> : null}
        <QueryStateWrapper
          data={sandboxAccounts}
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
      <OrganizationAccountDialog
        provider={editing?.provider ?? null}
        account={editing?.account}
        open={!!editing}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        onSaved={setNotice}
      />
    </>
  );
}
