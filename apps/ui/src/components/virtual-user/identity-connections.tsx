"use client";
import { ConnectionsPanel } from "@/components/connections/connections-panel";
export function IdentityConnections({ identityId }: { identityId: string }) {
  return <ConnectionsPanel identityId={identityId} />;
}
