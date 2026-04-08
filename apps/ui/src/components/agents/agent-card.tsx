"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Pencil } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { getDisplayName, getEntityNameClassName, getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";

interface AgentCardProps {
  agent: Agent;
  allCapabilities?: Capability[];
  showEditButton?: boolean;
  compact?: boolean;
}

export function AgentCard({
  agent,
  allCapabilities,
  showEditButton = false,
  compact = false,
}: AgentCardProps) {
  // Get capability info for display
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  // Capabilities are now directly on the agent
  const agentCapabilities = agent.capabilities ?? [];

  return (
    <Card className="bg-background transition-colors hover:bg-card">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex flex-col gap-0.5">
          <div className="flex items-center gap-2">
            <CardTitle className={`text-lg ${getEntityNameClassName(agent.status)}`}>
              <Link href={`/agents/${agent.id}`} className="hover:underline">
                {getDisplayName(agent)}
              </Link>
            </CardTitle>
            <CopyButton value={agent.id} />
          </div>
          <span className="text-xs text-muted-foreground font-mono">{agent.name}</span>
        </div>
        <Badge variant={getEntityStatusBadgeVariant(agent.status)}>{agent.status}</Badge>
      </CardHeader>
      <CardContent>
        {agent.description ? (
          <div className="text-sm text-muted-foreground mb-3 line-clamp-2">
            <InlineStreamdownMessage>{agent.description}</InlineStreamdownMessage>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground mb-3 italic">No description provided</p>
        )}

        {/* Capabilities display */}
        {agentCapabilities.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            <TooltipProvider>
              {agentCapabilities.map((capConfig) => {
                const cap = getCapabilityInfo(capConfig.ref);
                if (!cap) return null;
                const IconComponent = getCapabilityIcon(cap.icon);

                return (
                  <Tooltip key={capConfig.ref}>
                    <TooltipTrigger className="inline-flex cursor-default items-center gap-1 border bg-muted px-2 py-0.5 text-xs">
                      <IconComponent className="icon-sharp h-3 w-3" />
                      {!compact && <span>{cap.name}</span>}
                    </TooltipTrigger>
                    <TooltipContent>
                      <p className="font-medium">{cap.name}</p>
                      <p className="text-xs text-muted-foreground">{cap.description}</p>
                    </TooltipContent>
                  </Tooltip>
                );
              })}
            </TooltipProvider>
          </div>
        )}

        {/* Tags */}
        {agent.tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {agent.tags.map((tag) => (
              <Badge key={tag} variant="outline" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between">
          <span className="text-xs text-muted-foreground">
            Created {new Date(agent.created_at).toLocaleDateString()}
          </span>
          {showEditButton && agent.status === "active" && (
            <Link href={`/agents/${agent.id}/edit`}>
              <Button variant="ghost" size="icon" className="h-8 w-8">
                <Pencil className="icon-sharp h-4 w-4" />
              </Button>
            </Link>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
