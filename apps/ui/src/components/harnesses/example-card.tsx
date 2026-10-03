"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  EntityCard,
  EntityCardFooter,
  EntityCardTags,
  EntityCardDescription,
} from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Import } from "lucide-react";
import type { Capability, CapabilityId, HarnessExample } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import { HarnessIcon } from "@/lib/harness-icons";
import { IconTile } from "@/components/layout/page-layout";
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
  preview?: boolean;
}

export function HarnessExampleCard({
  example,
  allCapabilities,
  onImport,
  adopting = false,
  preview = false,
}: HarnessExampleCardProps) {
  const { locale } = useLocale();
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<HarnessIcon icon={example.icon} />} />}
      title={example.display_name}
      headerActions={
        <>
          {example.dev_only && (
            <Badge variant="outline" className="text-xs">
              dev
            </Badge>
          )}
          {preview && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onImport(example.name)}
              disabled={adopting}
            >
              {adopting ? "Importing..." : "Import"}
            </Button>
          )}
        </>
      }
      footer={
        !preview && (
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
        )
      }
    >
      <EntityCardDescription>{example.description}</EntityCardDescription>

      {!preview && (
        <div className="space-y-3">
          {example.capabilities.length > 0 && (
            <div className="flex flex-wrap gap-1">
              <TooltipProvider>
                {example.capabilities.map((capConfig) => {
                  const cap = getCapabilityInfo(capConfig.ref);
                  if (!cap) return null;
                  return (
                    <Tooltip key={capConfig.ref}>
                      <TooltipTrigger className="inline-flex cursor-default items-center gap-1.5 text-xs text-muted-foreground">
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

          <EntityCardTags tags={example.tags} />
        </div>
      )}
    </EntityCard>
  );
}
