"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { AgentIcon } from "@/components/icons/facet-icons";
import { useState } from "react";
import { useAgents, useCapabilities } from "@/hooks";
import { LinkButton } from "@/components/ui/button";
import { Plus } from "lucide-react";
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
  const ui = useUiTranslations();
  const [showArchived, setShowArchived] = useState(false);
  useChatPageFilters({ show_archived: showArchived });
  const { data: agents, isLoading, error } = useAgents({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Agents"), href: "/agents" }, { label: ui("All agents") }]}
      />

      <PageMasthead
        icon={<AgentIcon />}
        title={ui("All agents")}
        description={ui("Every agent definition in this organization.")}
        actions={
          <>
            <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
            <LinkButton variant="accent" href="/agents/new">
              <Plus className="size-4" />
              {ui("New agent")}
            </LinkButton>
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
                icon={<AgentIcon />}
                title={ui("No agents yet")}
                action={
                  <LinkButton variant="accent" href="/agents/new">
                    <Plus className="size-4" />
                    {ui("Create your first agent")}
                  </LinkButton>
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
        <BackLink href="/agents">{ui("Back to Agents")}</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
