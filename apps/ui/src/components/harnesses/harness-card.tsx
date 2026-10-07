"use client";

import Link from "next/link";
import { isHarnessDeprecated } from "@/lib/harness-deprecation";
import { Badge } from "@/components/ui/badge";
import { EntityStatus } from "@/components/ui/entity-status";
import { LinkButton } from "@/components/ui/button";
import {
  EntityCard,
  EntityCardFooter,
  EntityCardTags,
  EntityCardDescription,
  EntityCardCapabilities,
} from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { GitBranch, Pencil } from "lucide-react";
import { IconTile } from "@/components/layout/page-layout";
import type { Harness, Capability, CapabilityId } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import { HarnessIcon } from "@/lib/harness-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { getDisplayName, getEntityNameClassName } from "@/lib/entity-lifecycle";
import { formatCountLabel } from "@/lib/formatting";
import { normalizeTags } from "@/lib/tags";
import type { HarnessInheritance } from "@/lib/harness-inheritance";

interface HarnessCardProps {
  harness: Harness;
  allCapabilities?: Capability[];
  showEditButton?: boolean;
  compact?: boolean;
  inheritance?: HarnessInheritance;
}

export function HarnessCard({
  harness,
  allCapabilities,
  showEditButton = false,
  compact = false,
  inheritance,
}: HarnessCardProps) {
  const { locale } = useLocale();
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  const harnessCapabilities = harness.capabilities ?? [];
  const tags = normalizeTags(harness.tags);
  const sessionCount = harness.session_count ?? 0;
  const directParent = inheritance?.directParent;
  const directParentName = directParent ? getDisplayName(directParent) : null;
  const inheritanceSummary = inheritance?.hasCycle
    ? "Inheritance chain unavailable because it contains a cycle"
    : inheritance?.missingParentId
      ? `Parent harness unavailable (${inheritance.missingParentId})`
      : inheritance && inheritance.chain.length > 1
        ? `Inheritance chain: ${inheritance.chain.map(getDisplayName).join(" → ")}`
        : null;

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<HarnessIcon icon={harness.icon} />} />}
      title={getDisplayName(harness)}
      href={`/harnesses/${harness.id}`}
      titleClassName={getEntityNameClassName(harness.status)}
      headerActions={
        <>
          {harness.is_built_in && (
            <Badge variant="outline" className="text-xs">
              Built-in
            </Badge>
          )}
          {isHarnessDeprecated(harness) && <Badge variant="outline">Deprecated</Badge>}
          <EntityStatus status={harness.status} />
        </>
      }
      footer={
        <EntityCardFooter
          meta={
            <>
              <span>{formatCountLabel(sessionCount, "session")}</span>
            </>
          }
          actions={
            showEditButton &&
            !harness.is_built_in &&
            harness.status === "active" && (
              <LinkButton
                variant="ghost"
                size="icon"
                className="h-8 w-8"
                aria-label={`Edit ${getDisplayName(harness)}`}
                href={`/harnesses/${harness.id}?mode=edit`}
              >
                <Pencil className="icon-sharp h-4 w-4" />
              </LinkButton>
            )
          }
        />
      }
    >
      {harness.description ? (
        <EntityCardDescription>
          <InlineStreamdownMessage>{harness.description}</InlineStreamdownMessage>
        </EntityCardDescription>
      ) : (
        <p className="text-sm text-muted-foreground mb-3 italic">No description provided</p>
      )}

      {harness.parent_harness_id && (
        <div className="mb-3 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger
                aria-label="Show harness inheritance chain"
                className="inline-flex shrink-0 items-center"
              >
                <GitBranch className="icon-sharp size-3.5" />
              </TooltipTrigger>
              <TooltipContent>
                <p>{inheritanceSummary ?? "Direct parent harness"}</p>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
          <span className="shrink-0">Inherits from</span>
          {directParent && directParent.status !== "deleted" ? (
            <Link
              href={`/harnesses/${directParent.id}`}
              className="min-w-0 truncate font-medium text-foreground hover:underline"
              title={directParentName ?? undefined}
            >
              {directParentName}
            </Link>
          ) : (
            <span className="min-w-0 truncate italic" title={harness.parent_harness_id}>
              unavailable harness
            </span>
          )}
          {directParent?.status === "archived" && <span className="shrink-0">(archived)</span>}
        </div>
      )}

      <div className="space-y-3">
        {/* Capabilities display */}
        {harnessCapabilities.length > 0 && (
          <TooltipProvider>
            <EntityCardCapabilities
              label="Declared capabilities"
              ariaLabel="Locally declared capabilities"
            >
              {harnessCapabilities.map((capConfig) => {
                const cap = getCapabilityInfo(capConfig.ref);
                if (!cap) return null;
                return (
                  <Tooltip key={capConfig.ref}>
                    <TooltipTrigger className="inline-flex cursor-default items-center gap-1.5 text-xs text-muted-foreground">
                      <CapabilityIcon icon={cap.icon} className="icon-sharp h-3 w-3" />
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
            </EntityCardCapabilities>
          </TooltipProvider>
        )}

        <EntityCardTags tags={tags} />
      </div>
    </EntityCard>
  );
}
