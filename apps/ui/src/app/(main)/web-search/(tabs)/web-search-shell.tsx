"use client";

// Library → Web search: where agents' searches go. Providers lists every
// way the organization can search (model provider search, search accounts,
// free search); Defaults edits the organization's `web_search_defaults`.
// Mirrors the Models page: each tab is its own path (`/web-search`,
// `/web-search/defaults`), one "Connect" action in the masthead. This shell is
// the tabs' layout, so its dialogs and notices survive switching tabs. See knowledge/integrations/web-search.md (UI).

import { useMemo, useState, type ReactNode } from "react";
import { usePathname, useRouter } from "next/navigation";
import { Globe, Plus, SlidersHorizontal } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import { Skeleton } from "@/components/ui/skeleton";
import {
  PageBreadcrumb,
  PageContainer,
  PageMain,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { OrganizationAccountDialog } from "@/components/connections/organization-account-dialog";
import { SearchDefaultsTab } from "@/components/web-search/search-defaults-tab";
import { SearchProviderIcon } from "@/components/web-search/search-provider-icon";
import { SearchProvidersTab } from "@/components/web-search/search-providers-tab";
import { usePageTitle } from "@/hooks";
import { useProviders } from "@/hooks/use-providers";
import { useInvalidateSearchProviders, useSearchProviders } from "@/hooks/use-search-providers";
import { useConnectionProviders } from "@/hooks/use-user-connections";
import type { OrganizationConnection } from "@/lib/api/organization-connections";
import type { ConnectionProvider } from "@/lib/api/types";
import { tabFromPath, tabHref } from "@/lib/tab-routes";
import { useOrg } from "@/providers/org-provider";
import { usePageChatMode } from "@/components/chat-mode/chat-mode-context";

const TABS = ["providers", "defaults"] as const;
type Tab = (typeof TABS)[number];
const tabPath = (tab: Tab) => tabHref("/web-search", tab, "providers");
type PageNotice = { kind: "success" | "error"; text: string };

export function WebSearchShell({ children }: { children?: ReactNode }) {
  usePageTitle("Web search");
  const router = useRouter();
  const tab = tabFromPath(usePathname(), "/web-search", TABS, "providers");
  const { hasRole, currentOrg } = useOrg();
  const chatMode = usePageChatMode();
  const isAdmin = hasRole("admin");

  const catalog = useSearchProviders();
  const { data: modelProviders = [] } = useProviders();
  const { data: connectionProviders = [] } = useConnectionProviders();
  const invalidate = useInvalidateSearchProviders();

  // Connectors that hold search accounts, with their credential forms.
  const searchConnectors = useMemo(
    () =>
      connectionProviders.filter(
        (provider) =>
          provider.capabilities.includes("web_search") &&
          catalog.data?.connectors.some((entry) => entry.provider === provider.provider_id),
      ),
    [connectionProviders, catalog.data],
  );

  const [notice, setNotice] = useState<PageNotice | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [editing, setEditing] = useState<{
    provider: ConnectionProvider;
    account?: OrganizationConnection;
  } | null>(null);

  const setTab = (value: Tab) => router.push(tabPath(value));

  const connect = () => {
    if (searchConnectors.length === 1) setEditing({ provider: searchConnectors[0] });
    else setChoosing(true);
  };

  const providerCount = (catalog.data?.accounts.length ?? 0) + (catalog.data?.built_in.length ?? 0);

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Web search" }]} />
      <PageMasthead
        icon={<Globe />}
        title="Web search"
        description="Choose how agents search the web: built-in free search, the model provider's own search, or a search account you connect."
        actions={
          isAdmin ? (
            <Button variant="accent" onClick={connect} disabled={searchConnectors.length === 0}>
              <Plus className="size-4" />
              Connect search provider
            </Button>
          ) : undefined
        }
      />

      <SectionTabs
        aria-label="Web search sections"
        value={tab}
        items={[
          {
            value: "providers",
            href: tabPath("providers"),
            label: "Providers",
            icon: <Globe className="icon-sharp size-3.5" />,
            count: catalog.data ? providerCount : undefined,
          },
          {
            value: "defaults",
            href: tabPath("defaults"),
            label: "Defaults",
            icon: <SlidersHorizontal className="icon-sharp size-3.5" />,
          },
        ]}
      />

      {notice && (
        <Notice
          variant={notice.kind === "success" ? "success" : "destructive"}
          role={notice.kind === "success" ? "status" : "alert"}
        >
          <NoticeDescription className="flex items-center gap-3">
            <span className="flex-1">{notice.text}</span>
            <button
              type="button"
              className="text-xs text-muted-foreground hover:text-foreground"
              onClick={() => setNotice(null)}
            >
              Dismiss
            </button>
          </NoticeDescription>
        </Notice>
      )}

      <PageMain>
        {tab === "providers" && (
          <SearchProvidersTab
            catalog={catalog.data}
            isLoading={catalog.isLoading}
            error={catalog.error}
            modelProviders={modelProviders}
            isAdmin={isAdmin}
            onConnect={connect}
            onRotate={(account) => {
              const provider = connectionProviders.find(
                (candidate) => candidate.provider_id === account.provider,
              );
              if (!provider) return;
              setEditing({
                provider,
                account: {
                  id: account.id,
                  name: account.name,
                  provider: account.provider,
                  connected_at: account.connected_at,
                  updated_at: account.connected_at,
                },
              });
            }}
            onGoToDefaults={() => setTab("defaults")}
            onNotice={(next) => {
              setNotice(next);
              void invalidate();
            }}
          />
        )}
        {tab === "defaults" &&
          (catalog.data ? (
            <SearchDefaultsTab
              key={chatMode ? currentOrg?.public_id : JSON.stringify(catalog.data.defaults)}
              catalog={catalog.data}
              canManage={isAdmin}
            />
          ) : catalog.error ? (
            <p role="alert" className="text-sm text-destructive">
              Could not load the web search defaults.
            </p>
          ) : (
            <Skeleton className="h-96 max-w-4xl" />
          ))}
      </PageMain>

      <Dialog open={choosing} onOpenChange={setChoosing}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Connect a search provider</DialogTitle>
            <DialogDescription>
              The account belongs to the organization. Every agent can search with it, and its key
              never reaches an agent.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-2">
            {searchConnectors.map((provider) => (
              <button
                key={provider.provider_id}
                type="button"
                className="flex items-start gap-3 border p-3 text-left transition-colors hover:bg-muted/50"
                onClick={() => {
                  setChoosing(false);
                  setEditing({ provider });
                }}
              >
                <SearchProviderIcon
                  kind="account"
                  provider={provider.provider_id}
                  className="mt-0.5"
                />
                <span>
                  <span className="block text-sm font-medium">{provider.display_name}</span>
                  <span className="block text-xs text-muted-foreground">
                    {provider.description}
                  </span>
                </span>
              </button>
            ))}
          </div>
        </DialogContent>
      </Dialog>

      <OrganizationAccountDialog
        provider={editing?.provider ?? null}
        account={editing?.account}
        open={!!editing}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        description="A search account of this organization. Every agent can search with it; the key stays encrypted and never reaches an agent."
        onSaved={(message) => {
          setNotice({ kind: "success", text: `${message}. It joins the Automatic order.` });
          void invalidate();
          setTab("providers");
        }}
      />
      {children}
    </PageContainer>
  );
}
