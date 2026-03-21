"use client";

import { useState } from "react";
import { useAgents, useCapabilities } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus, ArrowLeft } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard } from "@/components/agents";
import { ArchiveFilter } from "@/components/archive-filter";

export default function AllAgentsPage() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: agents, isLoading, error } = useAgents({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Link href="/agents">
            <Button variant="ghost" size="icon" className="h-8 w-8">
              <ArrowLeft className="w-4 h-4" />
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">All Agents</h1>
        </div>
        <div className="flex items-center gap-2">
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Link href="/agents/new">
            <Button variant="accent">
              <Plus className="w-4 h-4 mr-2" />
              New Agent
            </Button>
          </Link>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={agents}
        errorMessagePrefix="Failed to load agents"
        emptyState={
          <div className="text-center py-12">
            <p className="text-muted-foreground mb-4">No agents yet</p>
            <Link href="/agents/new">
              <Button>
                <Plus className="w-4 h-4 mr-2" />
                Create your first agent
              </Button>
            </Link>
          </div>
        }
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((agent) => (
              <AgentCard
                key={agent.id}
                agent={agent}
                allCapabilities={allCapabilities}
                showEditButton
              />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}
