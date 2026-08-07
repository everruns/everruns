"use client";

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Pencil, Boxes } from "lucide-react";
import { IconTile } from "@/components/layout/page-layout";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";
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
  const { locale } = useLocale();
  // Get capability info for display
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  // Capabilities are now directly on the agent
  const agentCapabilities = agent.capabilities ?? [];
  const tags = normalizeTags(agent.tags);
  const sessionCount = agent.session_count ?? 0;
  const appCount = agent.app_count ?? 0;

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<Boxes />} />}
      title={getDisplayName(agent)}
      href={`/agents/${agent.id}`}
      titleClassName={getEntityNameClassName(agent.status)}
      copyValue={agent.id}
      subtitle={<span className="text-xs text-muted-foreground font-mono">{agent.name}</span>}
      headerActions={
        <Badge variant={getEntityStatusBadgeVariant(agent.status)}>{agent.status}</Badge>
      }
      footer={
        <EntityCardFooter
          meta={
            <>
              <span>Created {new Date(agent.created_at).toLocaleDateString()}</span>
              <span className="mx-2">·</span>
              <span>
                {formatCountLabel(sessionCount, "session")} · {formatCountLabel(appCount, "app")}
              </span>
            </>
          }
          actions={
            showEditButton &&
            agent.status === "active" && (
              <Link href={`/agents/${agent.id}/edit`}>
                <Button variant="ghost" size="icon" className="h-8 w-8">
                  <Pencil className="icon-sharp h-4 w-4" />
                </Button>
              </Link>
            )
          }
        />
      }
    >
      {agent.description ? (
        <div className="text-sm text-muted-foreground mb-3 line-clamp-2">
          <InlineStreamdownMessage>{agent.description}</InlineStreamdownMessage>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground mb-3 italic">No description provided</p>
      )}

      {/* Capabilities display */}
      {agentCapabilities.length > 0 && (
        <div className="flex flex-wrap gap-1 mb-3">
          <TooltipProvider>
            {agentCapabilities.map((capConfig) => {
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
