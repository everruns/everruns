"use client";

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Pencil, Shield } from "lucide-react";
import { IconTile } from "@/components/layout/page-layout";
import type { Harness, Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
} from "@/lib/entity-lifecycle";
import { formatCountLabel } from "@/lib/formatting";
import { normalizeTags } from "@/lib/tags";

interface HarnessCardProps {
  harness: Harness;
  allCapabilities?: Capability[];
  showEditButton?: boolean;
  compact?: boolean;
}

export function HarnessCard({
  harness,
  allCapabilities,
  showEditButton = false,
  compact = false,
}: HarnessCardProps) {
  const { locale } = useLocale();
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  const harnessCapabilities = harness.capabilities ?? [];
  const tags = normalizeTags(harness.tags);
  const sessionCount = harness.session_count ?? 0;
  const appCount = harness.app_count ?? 0;

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<Shield />} />}
      title={getDisplayName(harness)}
      href={`/harnesses/${harness.id}`}
      titleClassName={getEntityNameClassName(harness.status)}
      copyValue={harness.id}
      headerActions={
        <>
          {harness.is_built_in && (
            <Badge variant="outline" className="text-xs">
              Built-in
            </Badge>
          )}
          <Badge variant={getEntityStatusBadgeVariant(harness.status)}>{harness.status}</Badge>
        </>
      }
      footer={
        <EntityCardFooter
          meta={
            <>
              <span>Created {new Date(harness.created_at).toLocaleDateString()}</span>
              <span className="mx-2">·</span>
              <span>
                {formatCountLabel(sessionCount, "session")} · {formatCountLabel(appCount, "app")}
              </span>
            </>
          }
          actions={
            showEditButton &&
            !harness.is_built_in &&
            harness.status === "active" && (
              <Link href={`/harnesses/${harness.id}/edit`}>
                <Button variant="ghost" size="icon" className="h-8 w-8">
                  <Pencil className="icon-sharp h-4 w-4" />
                </Button>
              </Link>
            )
          }
        />
      }
    >
      {harness.description ? (
        <div className="text-sm text-muted-foreground mb-3 line-clamp-2">
          <InlineStreamdownMessage>{harness.description}</InlineStreamdownMessage>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground mb-3 italic">No description provided</p>
      )}

      {/* Capabilities display */}
      {harnessCapabilities.length > 0 && (
        <div className="flex flex-wrap gap-1 mb-3">
          <TooltipProvider>
            {harnessCapabilities.map((capConfig) => {
              const cap = getCapabilityInfo(capConfig.ref);
              if (!cap) return null;
              const IconComponent = getCapabilityIcon(cap.icon);

              return (
                <Tooltip key={capConfig.ref}>
                  <TooltipTrigger className="inline-flex cursor-default items-center gap-1 border bg-muted px-2 py-0.5 text-xs">
                    <IconComponent className="icon-sharp h-3 w-3" />
                    {!compact && <span>{localizedCapabilityName(cap, locale)}</span>}
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

      {/* Tags */}
      {tags.length > 0 && (
        <div className="flex flex-wrap gap-1 mb-3">
          {tags.map((tag) => (
            <Badge key={tag} variant="outline" className="text-xs">
              {tag}
            </Badge>
          ))}
        </div>
      )}
    </EntityCard>
  );
}
