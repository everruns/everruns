"use client";

// Settings tab: name, description, indexing mode, embedding model and search
// account. Admin-only, like the "Settings" tab link in the shell.

import { useUiTranslations } from "@/providers/locale-provider";
import { use } from "react";
import { PageMain } from "@/components/layout";
import { CollectionForm } from "@/components/knowledge/collection-form";
import {
  useCanManageKnowledge,
  useKnowledgeCollection,
  useRefreshKnowledge,
} from "@/components/knowledge/use-knowledge";
import { usePageTitle } from "@/hooks/use-page-title";
import { updateKnowledge } from "@/lib/api/knowledge";

export default function KnowledgeSettingsPage({
  params,
}: {
  params: Promise<{ collectionId: string }>;
}) {
  const ui = useUiTranslations();
  const { collectionId } = use(params);
  const collection = useKnowledgeCollection(collectionId);
  const canManage = useCanManageKnowledge();
  const refresh = useRefreshKnowledge();
  const c = collection.data;
  usePageTitle(ui("Settings"), c?.name ?? ui("Knowledge"));

  return (
    <PageMain>
      {!canManage ? (
        <p className="text-sm text-muted-foreground">
          {ui("Only organization owners and admins can change a collection.")}
        </p>
      ) : c ? (
        <div className="max-w-2xl border bg-card p-5">
          <CollectionForm
            key={c.id}
            initial={c}
            onSave={async (input) => {
              await updateKnowledge(collectionId, input);
              await refresh();
            }}
          />
        </div>
      ) : null}
    </PageMain>
  );
}
