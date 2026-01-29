"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Boxes, Plus } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";

interface AgentListWidgetProps {
  agents: Agent[];
  allCapabilities?: Capability[];
}

export function AgentListWidget({ agents, allCapabilities }: AgentListWidgetProps) {
  const activeAgents = agents.filter((a) => a.status === "active").slice(0, 5);

  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Active Agents</CardTitle>
        <Link href="/agents/new">
          <Button variant="accent" size="sm">
            <Plus className="h-4 w-4 mr-1" />
            New Agent
          </Button>
        </Link>
      </CardHeader>
      <CardContent>
        {activeAgents.length === 0 ? (
          <div className="text-center py-8">
            <Boxes className="h-12 w-12 mx-auto text-muted-foreground mb-2" />
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
                  className="flex items-center justify-between p-3 rounded-lg border hover:bg-accent transition-colors"
                >
                  <div className="flex items-center gap-3">
                    <Boxes className="h-5 w-5 text-muted-foreground" />
                    <div>
                      <div className="flex items-center gap-1">
                        <p className="font-medium">{agent.name}</p>
                        <CopyButton value={agent.id} />
                      </div>
                      <div className="flex items-center gap-2">
                        {/* Capabilities icons */}
                        {agentCapabilities.length > 0 && (
                          <TooltipProvider>
                            <div className="flex gap-0.5">
                              {agentCapabilities.slice(0, 3).map((capConfig) => {
                                const cap = getCapabilityInfo(capConfig.ref);
                                if (!cap) return null;
                                const IconComponent = getCapabilityIcon(cap.icon);

                                return (
                                  <Tooltip key={capConfig.ref}>
                                    <TooltipTrigger className="p-0.5 rounded bg-muted cursor-default">
                                      <IconComponent className="w-3 h-3 text-muted-foreground" />
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
                  <Badge variant="outline" className="bg-green-100 text-green-800">
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
