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
  Trash2,
  Pencil,
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  LucideIcon,
} from "lucide-react";
import type { Agent, AgentCapability, Capability } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
};

interface AgentCardProps {
  agent: Agent;
  capabilities?: AgentCapability[];
  allCapabilities?: Capability[];
  onDelete?: (agentId: string) => void;
  isDeleting?: boolean;
  showEditButton?: boolean;
  compact?: boolean;
}

export function AgentCard({
  agent,
  capabilities,
  allCapabilities,
  onDelete,
  isDeleting,
  showEditButton = false,
  compact = false,
}: AgentCardProps) {
  // Get capability info for display
  const getCapabilityInfo = (capabilityId: string): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  // Sort capabilities by position
  const sortedCapabilities = capabilities
    ? [...capabilities].sort((a, b) => a.position - b.position)
    : [];

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
        <p className="text-sm text-muted-foreground mb-3 line-clamp-2">
          {agent.description || "No description"}
        </p>

        {/* Capabilities display */}
        {sortedCapabilities.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            <TooltipProvider>
              {sortedCapabilities.map((ac) => {
                const cap = getCapabilityInfo(ac.capability_id);
                if (!cap) return null;
                const IconComponent = cap.icon
                  ? iconMap[cap.icon] || CircleOff
                  : CircleOff;

                return (
                  <Tooltip key={ac.capability_id}>
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
          <div className="flex gap-1">
            {showEditButton && (
              <Link href={`/agents/${agent.id}/edit`}>
                <Button variant="ghost" size="icon" className="h-8 w-8">
                  <Pencil className="w-4 h-4" />
                </Button>
              </Link>
            )}
            {onDelete && (
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-destructive hover:text-destructive"
                onClick={() => onDelete(agent.id)}
                disabled={isDeleting}
              >
                <Trash2 className="w-4 h-4" />
              </Button>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
