"use client";

import { useState } from "react";
import { useAgents, useCapabilities } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus, Boxes } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard } from "@/components/agents";
import { ArchiveFilter } from "@/components/archive-filter";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  EmptyState,
  PageColumns,
  PageMain,
  PageFooter,
  BackLink,
} from "@/components/layout";

export default function AgentsAllPageClient() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: agents, isLoading, error } = useAgents({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Agents", href: "/agents" }, { label: "All agents" }]} />

      <PageMasthead
        icon={<Boxes />}
        title="All agents"
        description="Every agent definition in this organization."
        actions={
          <>
            <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
            <Link href="/agents/new">
              <Button variant="accent">
                <Plus className="size-4" />
                New agent
              </Button>
            </Link>
          </>
        }
      />

      <PageColumns className="lg:grid-cols-1">
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={agents}
            errorMessagePrefix="Failed to load agents"
            emptyState={
              <EmptyState
                icon={<Boxes />}
                title="No agents yet"
                action={
                  <Link href="/agents/new">
                    <Button variant="accent">
                      <Plus className="size-4" />
                      Create your first agent
                    </Button>
                  </Link>
                }
              />
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {items.map((agent, index) => (
                  <AgentCard
                    key={agent.id ?? `agent-${index}`}
                    agent={agent}
                    allCapabilities={allCapabilities}
                    showEditButton
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </PageMain>
      </PageColumns>

      <PageFooter>
        <BackLink href="/agents">Back to Agents</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
