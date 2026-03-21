"use client";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Download } from "lucide-react";
import type { AgentTemplate, Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";

interface TemplateCardProps {
  template: AgentTemplate;
  allCapabilities?: Capability[];
  onInstall: (slug: string) => void;
  installing?: boolean;
}

export function TemplateCard({
  template,
  allCapabilities,
  onInstall,
  installing = false,
}: TemplateCardProps) {
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <Card className="bg-background transition-colors hover:bg-card">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <CardTitle className="text-lg">{template.name}</CardTitle>
        {template.dev_only && (
          <Badge variant="outline" className="text-xs">
            dev
          </Badge>
        )}
      </CardHeader>
      <CardContent>
        <p className="text-sm text-muted-foreground mb-3 line-clamp-2">{template.description}</p>

        {/* Capabilities display */}
        {template.capabilities.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            <TooltipProvider>
              {template.capabilities.map((capConfig) => {
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
        {template.tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {template.tags.map((tag) => (
              <Badge key={tag} variant="outline" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        {/* Install button */}
        <div className="flex items-center justify-end">
          <Button
            variant="accent"
            size="sm"
            onClick={() => onInstall(template.slug)}
            disabled={installing}
          >
            <Download className="w-4 h-4 mr-2" />
            {installing ? "Installing..." : "Install"}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
