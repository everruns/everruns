"use client";
import { useLocale } from "@/providers/locale-provider";
import { AgentIcon } from "@/components/icons/facet-icons";
import { Layers } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
import { EntityStatus } from "@/components/ui/entity-status";
import { IconTile } from "@/components/layout";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { CapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityCategory,
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { formatCountLabel } from "@/lib/formatting";
import type { Capability, CapabilityStatus } from "@/lib/api/types";
function getStatusLabel(status: CapabilityStatus): string {
  switch (status) {
    case "available":
      return "Available";
    case "coming_soon":
      return "Coming soon";
    case "deprecated":
      return "Deprecated";
    case "needs_identity":
      return "Needs identity";
    case "retired":
      return "Removed";
  }
}

function CapabilityStats({ capability }: { capability: Capability }) {
  const { displayLocale } = useLocale();
  const agents = capability.agent_count ?? 0;
  const harnesses = capability.harness_count ?? 0;
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span className="inline-flex items-center gap-1">
        <AgentIcon className="h-3.5 w-3.5" />
        {formatCountLabel(agents, "agent", undefined, displayLocale)}
      </span>
      <span className="inline-flex items-center gap-1">
        <Layers className="h-3.5 w-3.5" />
        {formatCountLabel(harnesses, "harness", "harnesses", displayLocale)}
      </span>
    </div>
  );
}

export function CapabilityCard({ capability }: { capability: Capability }) {
  const { locale } = useLocale();
  return (
    <EntityCard
      className="h-full"
      icon={<IconTile size="md" icon={<CapabilityIcon icon={capability.icon} />} />}
      title={localizedCapabilityName(capability, locale)}
      href={`/capabilities/${capability.id}`}
      headerActions={<EntityStatus status={getStatusLabel(capability.status)} />}
    >
      <EntityCardDescription>
        <InlineStreamdownMessage>
          {localizedCapabilityDescription(capability, locale)}
        </InlineStreamdownMessage>
      </EntityCardDescription>
      <div className="flex items-center justify-between gap-2 flex-wrap">
        {capability.category ? (
          <Badge variant="outline" className="text-xs">
            {localizedCapabilityCategory(capability.category, locale)}
          </Badge>
        ) : (
          <span />
        )}
        <CapabilityStats capability={capability} />
      </div>
    </EntityCard>
  );
}
