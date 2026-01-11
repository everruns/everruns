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
import {
  Pencil,
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  LucideIcon,
} from "lucide-react";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
};

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
    <Card className="hover:shadow-md transition-shadow">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="space-y-1">
          <CardTitle className="text-lg">
            <Link href={`/agents/${agent.id}`} className="hover:underline">
              {agent.name}
            </Link>
          </CardTitle>
          <p className="text-sm text-muted-foreground font-mono">
            {agent.id.slice(0, 8)}...
          </p>
        </div>
        <Badge variant={agent.status === "active" ? "default" : "secondary"}>
          {agent.status}
        </Badge>
      </CardHeader>
      <CardContent>
        <p className={`text-sm text-muted-foreground mb-3 line-clamp-2 ${!agent.description ? "italic" : ""}`}>
          {agent.description || "No description provided"}
        </p>

        {/* Capabilities display */}
        {agentCapabilities.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            <TooltipProvider>
              {agentCapabilities.map((capId) => {
                const cap = getCapabilityInfo(capId);
                if (!cap) return null;
                const IconComponent = cap.icon
                  ? iconMap[cap.icon] || CircleOff
                  : CircleOff;

                return (
                  <Tooltip key={capId}>
                    <TooltipTrigger className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-muted text-xs cursor-default">
                      <IconComponent className="w-3 h-3" />
                      {!compact && <span>{cap.name}</span>}
                    </TooltipTrigger>
                    <TooltipContent>
                      <p className="font-medium">{cap.name}</p>
                      <p className="text-xs text-muted-foreground">
                        {cap.description}
                      </p>
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
          {showEditButton && (
            <Link href={`/agents/${agent.id}/edit`}>
              <Button variant="ghost" size="icon" className="h-8 w-8">
                <Pencil className="w-4 h-4" />
              </Button>
            </Link>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
