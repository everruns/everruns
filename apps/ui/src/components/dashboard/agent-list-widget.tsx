"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Boxes, Plus } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { getCapabilityIcon } from "@/lib/capability-icons";

interface AgentListWidgetProps {
  agents: Agent[];
  allCapabilities?: Capability[];
}

const MAX_DISPLAYED = 5;

export function AgentListWidget({
  agents,
  allCapabilities,
}: AgentListWidgetProps) {
  const activeAgents = agents
    .filter((a) => a.status === "active")
    .slice(0, MAX_DISPLAYED);

  const getCapabilityInfo = (
    capabilityId: CapabilityId,
  ): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Active Agents</CardTitle>
        <Link href="/agents/new">
          <Button variant="accent" size="sm">
            <Plus className="icon-sharp mr-1 h-4 w-4" />
            New Agent
          </Button>
        </Link>
      </CardHeader>
      <CardContent>
        {activeAgents.length === 0 ? (
          <div className="text-center py-8">
            <Boxes className="icon-sharp mx-auto mb-2 h-12 w-12 text-muted-foreground" />
            <p className="text-muted-foreground">No agents yet.</p>
            <Link href="/agents/new">
              <Button variant="link">Create your first agent</Button>
            </Link>
          </div>
        ) : (
          <div className="space-y-3">
            {activeAgents.map((agent) => {
              // Capabilities are now directly on the agent
              const agentCapabilities = agent.capabilities ?? [];

              return (
                <Link
                  key={agent.id}
                  href={`/agents/${agent.id}`}
                  className="flex items-center justify-between border bg-card p-3 transition-colors hover:bg-muted/50"
                >
                  <div className="flex items-center gap-3">
                    <Boxes className="icon-sharp h-5 w-5 text-muted-foreground" />
                    <div>
                      <div className="flex items-center gap-1">
                        <p className="font-medium">{getDisplayName(agent)}</p>
                        <CopyButton value={agent.id} />
                      </div>
                      <div className="flex items-center gap-2">
                        {/* Capabilities icons */}
                        {agentCapabilities.length > 0 && (
                          <TooltipProvider>
                            <div className="flex gap-0.5">
                              {agentCapabilities
                                .slice(0, 3)
                                .map((capConfig) => {
                                  const cap = getCapabilityInfo(capConfig.ref);
                                  if (!cap) return null;
                                  const IconComponent = getCapabilityIcon(
                                    cap.icon,
                                  );

                                  return (
                                    <Tooltip key={capConfig.ref}>
                                      <TooltipTrigger className="cursor-default border bg-muted p-0.5">
                                        <IconComponent className="icon-sharp h-3 w-3 text-muted-foreground" />
                                      </TooltipTrigger>
                                      <TooltipContent>
                                        <p>{cap.name}</p>
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
                      </div>
                    </div>
                  </div>
                  <Badge
                    variant="outline"
                    className="border-border bg-muted text-foreground"
                  >
                    Active
                  </Badge>
                </Link>
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
