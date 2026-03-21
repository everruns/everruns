"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Plus } from "lucide-react";
import type { AgentExample, Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";

interface ExampleCardProps {
  example: AgentExample;
  allCapabilities?: Capability[];
  onUse: (slug: string) => void;
  adopting?: boolean;
}

export function ExampleCard({
  example,
  allCapabilities,
  onUse,
  adopting = false,
}: ExampleCardProps) {
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <Card className="bg-background transition-colors hover:bg-card">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <CardTitle className="text-lg">{example.name}</CardTitle>
        {example.dev_only && (
          <Badge variant="outline" className="text-xs">
            dev
          </Badge>
        )}
      </CardHeader>
      <CardContent>
        <p className="text-sm text-muted-foreground mb-3 line-clamp-2">{example.description}</p>

        {/* Capabilities display */}
        {example.capabilities.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            <TooltipProvider>
              {example.capabilities.map((capConfig) => {
                const cap = getCapabilityInfo(capConfig.ref);
                if (!cap) return null;
                const IconComponent = getCapabilityIcon(cap.icon);

                return (
                  <Tooltip key={capConfig.ref}>
                    <TooltipTrigger className="inline-flex cursor-default items-center gap-1 border bg-muted px-2 py-0.5 text-xs">
                      <IconComponent className="icon-sharp h-3 w-3" />
                      <span>{cap.name}</span>
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
        {example.tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {example.tags.map((tag) => (
              <Badge key={tag} variant="outline" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        {/* Use button */}
        <div className="flex items-center justify-end">
          <Button
            variant="accent"
            size="sm"
            onClick={() => onUse(example.slug)}
            disabled={adopting}
          >
            <Plus className="w-4 h-4 mr-2" />
            {adopting ? "Adding..." : "Use"}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
