"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { KeyRound } from "lucide-react";
import { ProviderAccountIcon } from "@/components/icons/facet-icons";
import { EmptyState, PageMain } from "@/components/layout";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { EntityStatus } from "@/components/ui/entity-status";
import { statusLabel, statusTone } from "@/components/knowledge/knowledge-status";
import {
  useKnowledgeCollections,
  useKnowledgeIndexers,
} from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";

// Accounts tab: the organization's search accounts (Turbopuffer) that indexed
// collections store their index in. The page header, tabs and "Add search
// account" action come from the shell (../knowledge-shell.tsx). Git sources
// need no account here: private GitHub repositories read through a service
// identity's GitHub connection, chosen when the source is connected.
export default function KnowledgeAccountsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Accounts"), ui("Knowledge"));
  const indexers = useKnowledgeIndexers();
  const collections = useKnowledgeCollections();
  const all = collections.data?.pages.flatMap((page) => page.data) ?? [];
  return (
    <PageMain className="space-y-6">
      <QueryStateWrapper
        data={indexers.data}
        isLoading={indexers.isLoading}
        error={indexers.error}
        errorMessagePrefix="Unable to load search accounts"
        emptyState={
          <EmptyState
            icon={<ProviderAccountIcon />}
            title={ui("No search accounts")}
            description={ui(
              "Collections index into the deployment's default search backend when one is configured. Add a Turbopuffer account to keep an index in your own account.",
            )}
          />
        }
      >
        {(accounts) => (
          <div className="grid gap-4 xl:grid-cols-2">
            {accounts.map((account) => {
              const users = all.filter((collection) => collection.indexer_id === account.id);
              return (
                <EntityCard
                  key={account.id}
                  icon={<KeyRound className="icon-sharp size-5 text-muted-foreground" />}
                  title={account.name}
                  subtitle={account.endpoint.replace(/^https:\/\//, "")}
                  headerActions={
                    <EntityStatus
                      status={account.status}
                      tone={statusTone(account.status)}
                      label={statusLabel(account.status)}
                    />
                  }
                  footer={
                    <EntityCardFooter
                      meta={
                        users.length
                          ? ui("Used by {value0}", {
                              value0: users.map((collection) => collection.name).join(", "),
                            })
                          : ui("Not used by a collection yet")
                      }
                    />
                  }
                />
              );
            })}
          </div>
        )}
      </QueryStateWrapper>
      <p className="max-w-3xl text-sm text-muted-foreground">
        {ui(
          "Git sources need no account here. Public repositories are read anonymously; private GitHub repositories read through a service identity's GitHub connection, chosen when you connect the source.",
        )}
      </p>
    </PageMain>
  );
}
