"use client";
import { usePageTitle } from "@/hooks";
import { ConnectionsPanel } from "@/components/connections/connections-panel";
import { PendingConnectionMigrations } from "@/components/connections/pending-connection-migrations";
export default function ConnectionsPage() {
  usePageTitle("Connections", "Settings");
  return (
    <div className="space-y-6">
      <PendingConnectionMigrations />
      <ConnectionsPanel />
    </div>
  );
}
