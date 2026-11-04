"use client";
import { ConnectionsPanel } from "@/components/connections/connections-panel";
import { ScopedCredentialsPanel } from "@/components/credentials/scoped-credentials-panel";
export function IdentityConnections({
  identityId,
  usage,
}: {
  identityId: string;
  usage: "service" | "end_user";
}) {
  return (
    <div className="space-y-6">
      <ConnectionsPanel identityId={identityId} />
      <ScopedCredentialsPanel
        owner={{
          scope: "virtual_user",
          owner_id: identityId,
          usage: usage === "end_user" ? "user" : "service",
        }}
      />
    </div>
  );
}
