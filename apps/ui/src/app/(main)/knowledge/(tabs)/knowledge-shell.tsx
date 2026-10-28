"use client";

// Knowledge: one page with tabs, organized like Sandboxes. The collections
// agents mount, and the organization accounts collections index into. Each tab
// is its own path (`/knowledge`, `/knowledge/accounts`); the shell is the
// tabs' layout and each tab's page renders its content.

import { useUiTranslations } from "@/providers/locale-provider";
import { useState, type ReactNode } from "react";
import { usePathname, useRouter } from "next/navigation";
import { FolderOpen, KeyRound, Plus } from "lucide-react";
import { KnowledgeIcon } from "@/components/icons/facet-icons";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { PageBreadcrumb, PageContainer, PageMasthead, SectionTabs } from "@/components/layout";
import { CollectionForm } from "@/components/knowledge/collection-form";
import { IndexerForm } from "@/components/knowledge/indexer-form";
import {
  useCanManageKnowledge,
  useKnowledgeCollections,
  useKnowledgeIndexers,
  useRefreshKnowledge,
} from "@/components/knowledge/use-knowledge";
import { createKnowledge } from "@/lib/api/knowledge";
import { tabFromPath, tabHref } from "@/lib/tab-routes";

const TABS = ["collections", "accounts"] as const;
type Tab = (typeof TABS)[number];
const tabPath = (tab: Tab) => tabHref("/knowledge", tab, "collections");

const DESCRIPTIONS: Record<Tab, string> = {
  collections: "Folders of files agents can mount, read and grep. Indexing adds passage search.",
  accounts: "Organization search accounts that indexed collections store their index in.",
};

export function KnowledgeShell({ children }: { children: ReactNode }) {
  const ui = useUiTranslations();
  const router = useRouter();
  const tab = tabFromPath(usePathname(), "/knowledge", TABS, "collections");
  const canManage = useCanManageKnowledge();
  const collections = useKnowledgeCollections();
  const indexers = useKnowledgeIndexers();
  const refresh = useRefreshKnowledge();
  const [dialog, setDialog] = useState<"collection" | "account" | null>(null);
  const loaded = collections.data?.pages.flatMap((page) => page.data);

  const tabs = [
    {
      value: "collections",
      href: tabPath("collections"),
      label: ui("Collections"),
      icon: <FolderOpen className="icon-sharp size-3.5" />,
      count: collections.hasNextPage ? undefined : loaded?.length,
    },
    // Search accounts hold organization credentials; only admins manage them.
    ...(canManage || tab === "accounts"
      ? [
          {
            value: "accounts",
            href: tabPath("accounts"),
            label: ui("Accounts"),
            icon: <KeyRound className="icon-sharp size-3.5" />,
            count: indexers.data?.length,
          },
        ]
      : []),
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Knowledge") }]} />
      <PageMasthead
        icon={<KnowledgeIcon />}
        title={ui("Knowledge")}
        description={ui(DESCRIPTIONS[tab])}
        actions={
          !canManage ? undefined : tab === "accounts" ? (
            <Button variant="accent" onClick={() => setDialog("account")}>
              <Plus className="size-4" /> {ui("Add search account")}
            </Button>
          ) : (
            <Button variant="accent" onClick={() => setDialog("collection")}>
              <Plus className="size-4" /> {ui("New collection")}
            </Button>
          )
        }
      />
      <SectionTabs aria-label={ui("Knowledge sections")} value={tab} items={tabs} />
      {children}

      <Dialog open={dialog === "collection"} onOpenChange={(open) => !open && setDialog(null)}>
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>{ui("New collection")}</DialogTitle>
            <DialogDescription>
              {ui("Start with files only. Indexing can be turned on later.")}
            </DialogDescription>
          </DialogHeader>
          <CollectionForm
            onCancel={() => setDialog(null)}
            onSave={async (input) => {
              const created = await createKnowledge(input);
              setDialog(null);
              await refresh();
              router.push(`/knowledge/${created.id}`);
            }}
          />
        </DialogContent>
      </Dialog>

      <Dialog open={dialog === "account"} onOpenChange={(open) => !open && setDialog(null)}>
        <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>{ui("Add search account")}</DialogTitle>
          </DialogHeader>
          <IndexerForm
            onSaved={async () => {
              setDialog(null);
              await refresh();
            }}
          />
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}
