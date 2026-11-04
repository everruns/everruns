"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePageTitle } from "@/hooks";
import { ConnectionsPanel } from "@/components/connections/connections-panel";
import { PendingConnectionMigrations } from "@/components/connections/pending-connection-migrations";
import { ScopedCredentialsPanel } from "@/components/credentials/scoped-credentials-panel";

// OAuth and API-key setup return to this URL (`?connected=` or `?connect_error=`).
export default function ConnectionsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Connections"), ui("Settings"));
  return (
    <div className="space-y-8">
      <PendingConnectionMigrations />
      <ConnectionsPanel />
      <ScopedCredentialsPanel owner={{ scope: "virtual_user", owner_id: "me", usage: "user" }} />
    </div>
  );
}
