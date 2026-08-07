"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Import } from "lucide-react";
import type { Capability, CapabilityId, HarnessExample } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";

interface HarnessExampleCardProps {
  example: HarnessExample;
  allCapabilities?: Capability[];
  onImport: (name: string) => void;
  adopting?: boolean;
}

export function HarnessExampleCard({
  example,
  allCapabilities,
  onImport,
  adopting = false,
}: HarnessExampleCardProps) {
  const { locale } = useLocale();
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <EntityCard
      title={example.display_name}
      headerActions={
        example.dev_only && (
          <Badge variant="outline" className="text-xs">
            dev
          </Badge>
        )
      }
      footer={
        <EntityCardFooter
          actions={
            <Button
              variant="accent"
              size="sm"
              onClick={() => onImport(example.name)}
              disabled={adopting}
            >
              <Import className="w-4 h-4 mr-2" />
              {adopting ? "Importing..." : "Import"}
            </Button>
          }
        />
      }
    >
      <p className="text-sm text-muted-foreground mb-3 line-clamp-2">{example.description}</p>

      {example.capabilities.length > 0 && (
        <div className="flex flex-wrap gap-1 mb-3">
          <TooltipProvider>
            {example.capabilities.map((capConfig) => {
              const cap = getCapabilityInfo(capConfig.ref);
              if (!cap) return null;
              return (
                <Tooltip key={capConfig.ref}>
                  <TooltipTrigger className="inline-flex cursor-default items-center gap-1 border bg-muted px-2 py-0.5 text-xs">
                    <CapabilityIcon icon={cap.icon} className="icon-sharp h-3 w-3" />
                    <span>{localizedCapabilityName(cap, locale)}</span>
                  </TooltipTrigger>
                  <TooltipContent>
                    <p className="font-medium">{localizedCapabilityName(cap, locale)}</p>
                    <p className="text-xs text-muted-foreground">
                      {localizedCapabilityDescription(cap, locale)}
                    </p>
                  </TooltipContent>
                </Tooltip>
              );
            })}
          </TooltipProvider>
        </div>
      )}

      {example.tags.length > 0 && (
        <div className="flex flex-wrap gap-1 mb-3">
          {example.tags.map((tag) => (
            <Badge key={tag} variant="outline" className="text-xs">
              {tag}
            </Badge>
          ))}
        </div>
      )}
    </EntityCard>
  );
}
