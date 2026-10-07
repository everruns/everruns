"use client";

import Link from "next/link";
import { EntityStatus } from "@/components/ui/entity-status";
import { LinkButton } from "@/components/ui/button";
import {
  EntityCard,
  EntityCardDetail,
  EntityCardFooter,
  EntityCardTags,
  EntityCardDescription,
  EntityCardCapabilities,
} from "@/components/ui/entity-card";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Pencil, Shield } from "lucide-react";
import { IconTile } from "@/components/layout/page-layout";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import type { Agent, Capability, CapabilityId } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { getDisplayName, getEntityNameClassName } from "@/lib/entity-lifecycle";
import { formatCountLabel } from "@/lib/formatting";
import { normalizeTags } from "@/lib/tags";
import { AgentCardChannels } from "./agent-card-channels";

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
  const harness = agent.effective_harness;
  const harnessName = harness?.display_name || harness?.name;
  const harnessSourceLabel =
    harness?.source === "organization_default" ? "organization default" : "explicit";
  const harnessStatusLabel = harness?.status === "unresolved" ? "unavailable" : harness?.status;
  const harnessTooltip = harnessName
    ? `Harness: ${harnessName} (${harnessSourceLabel}${harnessStatusLabel && harnessStatusLabel !== "active" ? `, ${harnessStatusLabel}` : ""})`
    : `Harness: unavailable (${harnessSourceLabel})`;
  const harnessIsLinked =
    harness?.id && harness.status !== "deleted" && harness.status !== "unresolved";

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<AgentAvatar avatar={agent.avatar} size={28} />} />}
      title={getDisplayName(agent)}
      href={`/agents/${agent.id}`}
      titleClassName={getEntityNameClassName(agent.status)}
      headerActions={<EntityStatus status={agent.status} />}
      footer={
        <>
          <EntityCardFooter
            meta={
              <>
                <span>{formatCountLabel(sessionCount, "session")}</span>
              </>
            }
            actions={
              showEditButton &&
              agent.status === "active" &&
              !agent.is_built_in && (
                <LinkButton
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8"
                  aria-label={`Edit ${getDisplayName(agent)}`}
                  href={`/agents/${agent.id}?mode=edit`}
                >
                  <Pencil className="icon-sharp h-4 w-4" />
                </LinkButton>
              )
            }
          />
        </>
      }
    >
      {agent.description ? (
        <EntityCardDescription>
          <InlineStreamdownMessage>{agent.description}</InlineStreamdownMessage>
        </EntityCardDescription>
      ) : (
        <p className="text-sm text-muted-foreground mb-3 italic">No description provided</p>
      )}

      <EntityCardDetail
        className="mb-3"
        label="Harness"
        icon={
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger
                aria-label={harnessTooltip}
                className="inline-flex cursor-help items-center"
              >
                <Shield className="icon-sharp size-3.5" />
              </TooltipTrigger>
              <TooltipContent>{harnessTooltip}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        }
      >
        {harnessIsLinked ? (
          <Link
            href={`/harnesses/${harness.id}`}
            className="min-w-0 truncate font-medium text-foreground hover:underline"
            title={harnessName ?? undefined}
            aria-label={harnessTooltip}
          >
            {harnessName}
          </Link>
        ) : (
          <span
            className="min-w-0 truncate italic"
            title={harness?.id ?? "Harness could not be resolved"}
          >
            {harnessName ? `${harnessName} (${harnessStatusLabel})` : "unavailable harness"}
          </span>
        )}
        <span className="shrink-0 text-muted-foreground">
          {harness?.source === "organization_default" ? "Org default" : "Explicit"}
        </span>
      </EntityCardDetail>

      <div className="space-y-3">
        {/* Capabilities display */}
        {agentCapabilities.length > 0 && (
          <TooltipProvider>
            <EntityCardCapabilities>
              {agentCapabilities.map((capConfig) => {
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

        <AgentCardChannels agent={agent} canManage={showEditButton} />
        <EntityCardTags tags={tags} />
      </div>
    </EntityCard>
  );
}
