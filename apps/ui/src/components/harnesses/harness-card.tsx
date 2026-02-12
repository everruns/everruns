"use client";

import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Pencil } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Harness, Capability, CapabilityId } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";

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
  const getCapabilityInfo = (capabilityId: CapabilityId): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  const harnessCapabilities = harness.capabilities ?? [];

  return (
    <Card className="hover:shadow-md transition-shadow">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-2">
          <CardTitle className="text-lg">
            <Link href={`/harnesses/${harness.id}`} className="hover:underline">
              {harness.name}
            </Link>
          </CardTitle>
          <CopyButton value={harness.id} />
        </div>
        <Badge variant={harness.status === "active" ? "default" : "secondary"}>
          {harness.status}
        </Badge>
      </CardHeader>
      <CardContent>
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
                    <TooltipTrigger className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-muted text-xs cursor-default">
                      <IconComponent className="w-3 h-3" />
                      {!compact && <span>{cap.name}</span>}
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
        {harness.tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {harness.tags.map((tag) => (
              <Badge key={tag} variant="outline" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between">
          <span className="text-xs text-muted-foreground">
            Created {new Date(harness.created_at).toLocaleDateString()}
          </span>
          {showEditButton && (
            <Link href={`/harnesses/${harness.id}/edit`}>
              <Button variant="ghost" size="icon" className="h-8 w-8">
                <Pencil className="w-4 h-4" />
              </Button>
            </Link>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
