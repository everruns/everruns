"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { harnessChoiceLabel } from "@/lib/harness-deprecation";
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
import type { Capability, CapabilityId } from "@/lib/api/types";
import type { GuidedAgentExample } from "@/lib/api/agent-examples";
import { CapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { localizedEntityDescription } from "@/lib/entity-localization";

interface ExampleCardProps {
  example: GuidedAgentExample;
  allCapabilities?: Capability[];
  onImport: (name: string) => void;
  adopting?: boolean;
  preview?: boolean;
}

export function ExampleCard({
  example,
  allCapabilities,
  onImport,
  adopting = false,
  preview = false,
}: ExampleCardProps) {
  const ui = useUiTranslations();
  const { locale, displayLocale } = useLocale();
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  return (
    <EntityCard
      title={getDisplayName(example, displayLocale)}
      headerActions={
        <>
          {(example.dev_only || example.setup) && (
            <div className="flex gap-1">
              {example.setup && (
                <Badge variant="outline" className="text-xs">
                  {ui("guided setup")}
                </Badge>
              )}
              {example.dev_only && (
                <Badge variant="outline" className="text-xs">
                  {ui("dev")}
                </Badge>
              )}
            </div>
          )}
          {preview && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onImport(example.name)}
              disabled={adopting}
            >
              {adopting ? ui("Importing...") : ui("Import")}
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
                {adopting ? ui("Importing...") : ui("Import")}
              </Button>
            }
          />
        )
      }
    >
      <p className="mb-2 text-xs text-muted-foreground">
        {ui("Harness:")} {harnessChoiceLabel(example.harness_name)}
      </p>
      <EntityCardDescription>
        {localizedEntityDescription(example, displayLocale)}
      </EntityCardDescription>

      {!preview && (
        <div className="space-y-3">
          {/* Capabilities display */}
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
