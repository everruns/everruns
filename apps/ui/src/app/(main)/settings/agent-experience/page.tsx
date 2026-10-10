"use client";
import { useState } from "react";
import Link from "next/link";
import { useVirtualUser, useUpdateVirtualUser } from "@/hooks/use-virtual-users";
import { usePageTitle } from "@/hooks";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Combobox } from "@/components/ui/combobox";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "@/lib/locale-data";
import { ConnectionsPanel } from "@/components/connections/connections-panel";
import { PendingConnectionMigrations } from "@/components/connections/pending-connection-migrations";
import { UserMcpServersPanel } from "@/components/connections/user-mcp-servers-panel";
import { McpGrantsPanel } from "@/components/connections/mcp-grants-panel";
import { ConnectedClientsPanel } from "@/components/connections/connected-clients-panel";
import type { UpdateVirtualUserRequest } from "@/lib/api/types";
export default function AgentExperiencePage() {
  usePageTitle("My agent experience", "Settings");
  const profile = useVirtualUser("me");
  const update = useUpdateVirtualUser();
  const [changes, setChanges] = useState<UpdateVirtualUserRequest>({});
  if (profile.isLoading) return <p>Loading your agent experience…</p>;
  if (!profile.data)
    return <p className="text-destructive">Your virtual user could not be loaded.</p>;
  const user = profile.data;
  return (
    <div className="space-y-8">
      <div>
        <h2 className="text-xl font-semibold">My agent experience</h2>
        <p className="text-sm text-muted-foreground">
          Your profile, MCP servers and connections for chats in this organization.
        </p>
        <Link className="text-sm text-primary underline" href={`/virtual-users/${user.id}`}>
          View virtual user
        </Link>
      </div>
      <form
        className="space-y-4"
        onSubmit={async (e) => {
          e.preventDefault();
          await update.mutateAsync({ identityId: "me", request: changes });
          setChanges({});
        }}
      >
        <div className="space-y-2">
          <Label htmlFor="runtime-name">Name</Label>
          <Input
            id="runtime-name"
            required
            value={changes.name ?? user.name}
            onChange={(e) => setChanges({ ...changes, name: e.target.value })}
          />
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-2">
            <Label>Locale</Label>
            <Combobox
              options={LOCALE_OPTIONS}
              value={changes.locale === undefined ? (user.locale ?? "") : (changes.locale ?? "")}
              onValueChange={(value) => setChanges({ ...changes, locale: value || null })}
            />
          </div>
          <div className="space-y-2">
            <Label>Timezone</Label>
            <Combobox
              options={TIMEZONE_OPTIONS}
              value={
                changes.timezone === undefined ? (user.timezone ?? "") : (changes.timezone ?? "")
              }
              onValueChange={(value) => setChanges({ ...changes, timezone: value || null })}
            />
          </div>
        </div>
        <Button disabled={update.isPending} type="submit">
          Save profile
        </Button>
        {update.error && <p className="text-destructive">{update.error.message}</p>}
      </form>
      <PendingConnectionMigrations />
      <UserMcpServersPanel />
      <ConnectedClientsPanel />
      <McpGrantsPanel />
      <ConnectionsPanel />
    </div>
  );
}
