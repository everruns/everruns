"use client";

import { usePageTitle } from "@/hooks";
import { ConnectionsPanel } from "@/components/connections/connections-panel";
import { PendingConnectionMigrations } from "@/components/connections/pending-connection-migrations";
import { ScopedCredentialsPanel } from "@/components/credentials/scoped-credentials-panel";

// OAuth and API-key setup return to this URL (`?connected=` or `?connect_error=`).
export default function ConnectionsPage() {
  usePageTitle("Connections", "Settings");
  return (
    <div className="space-y-8">
      <PendingConnectionMigrations />
      <ConnectionsPanel />
      <ScopedCredentialsPanel owner={{ scope: "virtual_user", owner_id: "me", usage: "user" }} />
    </div>
  );
}
