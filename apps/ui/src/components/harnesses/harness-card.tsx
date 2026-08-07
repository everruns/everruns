"use client";

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { GitBranch, Pencil, Shield } from "lucide-react";
import { IconTile } from "@/components/layout/page-layout";
import type { Harness, Capability, CapabilityId } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
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
  const appCount = harness.app_count ?? 0;
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

      {/* Capabilities display */}
      {harnessCapabilities.length > 0 && (
        <div
          className="flex flex-wrap items-center gap-1 mb-3"
          aria-label="Locally declared capabilities"
        >
          <span className="mr-1 text-[11px] text-muted-foreground">Declared capabilities</span>
          <TooltipProvider>
            {harnessCapabilities.map((capConfig) => {
              const cap = getCapabilityInfo(capConfig.ref);
              if (!cap) return null;
              return (
                <Tooltip key={capConfig.ref}>
                  <TooltipTrigger className="inline-flex cursor-default items-center gap-1 border bg-muted px-2 py-0.5 text-xs">
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
