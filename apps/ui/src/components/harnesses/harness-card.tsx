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
    <Card className="bg-background transition-colors hover:bg-card">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-2">
          <CardTitle className="text-lg">
            <Link href={`/harnesses/${harness.id}`} className="hover:underline">
              {harness.name}
            </Link>
          </CardTitle>
          <CopyButton value={harness.id} />
        </div>
        <div className="flex items-center gap-1">
          {harness.is_built_in && (
            <Badge variant="outline" className="text-xs">
              Built-in
            </Badge>
          )}
          <Badge variant={harness.status === "active" ? "default" : "secondary"}>
            {harness.status}
          </Badge>
        </div>
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
                    <TooltipTrigger className="inline-flex cursor-default items-center gap-1 border bg-muted px-2 py-0.5 text-xs">
                      <IconComponent className="icon-sharp h-3 w-3" />
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
          {showEditButton && !harness.is_built_in && (
            <Link href={`/harnesses/${harness.id}/edit`}>
              <Button variant="ghost" size="icon" className="h-8 w-8">
                <Pencil className="icon-sharp h-4 w-4" />
              </Button>
            </Link>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
