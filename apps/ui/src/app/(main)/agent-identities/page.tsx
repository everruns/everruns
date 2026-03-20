"use client";

import Link from "next/link";
import { useState } from "react";
import { Plus, UserRound } from "lucide-react";
import { ArchiveFilter } from "@/components/archive-filter";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/ui/copy-button";
import { useAgentIdentities } from "@/hooks/use-agent-identities";
import { getEntityNameClassName, getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";

export default function AgentIdentitiesPage() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: identities, isLoading, error } = useAgentIdentities({ includeArchived: showArchived });

  if (error) {
    return <div className="container mx-auto p-6 text-red-500">Error loading identities: {error.message}</div>;
  }

  return (
    <div className="container mx-auto p-6">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-2xl font-bold">Agent Identities</h1>
        <div className="flex items-center gap-2">
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Link href="/agent-identities/new">
            <Button variant="accent"><Plus className="mr-2 h-4 w-4" />New Identity</Button>
          </Link>
        </div>
      </div>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">{[...Array(3)].map((_, i) => <Card key={i}><CardHeader><Skeleton className="h-6 w-1/2" /></CardHeader><CardContent><Skeleton className="h-4 w-full" /></CardContent></Card>)}</div>
      ) : identities && identities.length > 0 ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {identities.map((identity) => (
            <Link key={identity.id} href={`/agent-identities/${identity.id}`}>
              <Card className="cursor-pointer hover:border-accent/50 transition-colors">
                <CardHeader>
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex items-center gap-2">
                      <UserRound className="h-5 w-5 text-muted-foreground" />
                      <h3 className={`text-lg font-semibold ${getEntityNameClassName(identity.status)}`}>{identity.name}</h3>
                      <CopyButton value={identity.id} />
                    </div>
                    <Badge variant={getEntityStatusBadgeVariant(identity.status)}>{identity.status}</Badge>
                  </div>
                </CardHeader>
                <CardContent className="space-y-2 text-sm text-muted-foreground">
                  {identity.description && <p>{identity.description}</p>}
                  <p>{identity.locale || "No locale"} · {identity.timezone || "No timezone"}</p>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      ) : (
        <div className="py-12 text-center text-muted-foreground">No agent identities yet.</div>
      )}
    </div>
  );
}
