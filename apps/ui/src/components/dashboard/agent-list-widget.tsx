"use client";

import Link from "next/link";
import { NewAgentLink } from "@/components/dashboard/new-agent-link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { EntityCard } from "@/components/ui/entity-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Boxes, Plus } from "lucide-react";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { CapabilityIcon } from "@/lib/capability-icons";
import { localizedCapabilityName } from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";

interface AgentListWidgetProps {
  agents: Agent[];
  allCapabilities?: Capability[];
}

// Number of active agents rendered in the widget. Exported so the dashboard can
// gate the capability-catalog fetch on exactly the agents it will display.
export const MAX_DISPLAYED_AGENTS = 5;

export function AgentListWidget({ agents, allCapabilities }: AgentListWidgetProps) {
  const { locale } = useLocale();
  const activeAgents = agents.filter((a) => a.status === "active").slice(0, MAX_DISPLAYED_AGENTS);

  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Active Agents</CardTitle>
        <NewAgentLink>
          <Button variant="accent" size="sm">
            <Plus className="icon-sharp mr-1 h-4 w-4" />
            New Agent
          </Button>
        </NewAgentLink>
      </CardHeader>
      <CardContent>
        {activeAgents.length === 0 ? (
          <div className="text-center py-8">
            <Boxes className="icon-sharp mx-auto mb-2 h-12 w-12 text-muted-foreground" />
            <p className="text-muted-foreground">No agents yet.</p>
            <NewAgentLink>
              <Button variant="link">Create your first agent</Button>
            </NewAgentLink>
          </div>
        ) : (
          <div className="space-y-3">
            {activeAgents.map((agent) => {
              // Capabilities are now directly on the agent
              const agentCapabilities = agent.capabilities ?? [];

              return (
                <EntityCard
                  key={agent.id}
                  variant="row"
                  href={`/agents/${agent.id}`}
                  title={getDisplayName(agent)}
                  copyValue={agent.id}
                  icon={<Boxes className="icon-sharp h-5 w-5 text-muted-foreground" />}
                  headerActions={
                    <Badge variant="outline" className="border-border bg-muted text-foreground">
                      Active
                    </Badge>
                  }
                >
                  {/* Capabilities icons */}
                  {agentCapabilities.length > 0 && (
                    <TooltipProvider>
                      <div className="mt-1 flex gap-0.5">
                        {agentCapabilities.slice(0, 3).map((capConfig) => {
                          const cap = getCapabilityInfo(capConfig.ref);
                          if (!cap) return null;
                          return (
                            <Tooltip key={capConfig.ref}>
                              <TooltipTrigger className="cursor-default border bg-muted p-0.5">
                                <CapabilityIcon
                                  icon={cap.icon}
                                  className="icon-sharp h-3 w-3 text-muted-foreground"
                                />
                              </TooltipTrigger>
                              <TooltipContent>
                                <p>{localizedCapabilityName(cap, locale)}</p>
                              </TooltipContent>
                            </Tooltip>
                          );
                        })}
                        {agentCapabilities.length > 3 && (
                          <span className="text-xs text-muted-foreground ml-1">
                            +{agentCapabilities.length - 3}
                          </span>
                        )}
                      </div>
                    </TooltipProvider>
                  )}
                </EntityCard>
              );
            })}
            <Link
              href="/agents"
              className="block text-center text-sm text-muted-foreground hover:text-foreground hover:underline pt-2"
            >
              View all agents →
            </Link>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
