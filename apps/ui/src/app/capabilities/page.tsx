"use client";

import { useCapabilities } from "@/hooks";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  CircleOff,
  Search,
  Box,
  Folder,
  LucideIcon,
} from "lucide-react";
import type { Capability, CapabilityStatus } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  search: Search,
  box: Box,
  folder: Folder,
};

function getStatusBadgeVariant(
  status: CapabilityStatus
): "default" | "secondary" | "outline" {
  switch (status) {
    case "available":
      return "default";
    case "coming_soon":
      return "secondary";
    case "deprecated":
      return "outline";
  }
}

function getStatusLabel(status: CapabilityStatus): string {
  switch (status) {
    case "available":
      return "Available";
    case "coming_soon":
      return "Coming Soon";
    case "deprecated":
      return "Deprecated";
  }
}

function CapabilityCard({ capability }: { capability: Capability }) {
  const IconComponent = capability.icon
    ? iconMap[capability.icon] || CircleOff
    : CircleOff;

  return (
    <Card className="hover:shadow-md transition-shadow">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <div className="p-2 bg-muted rounded-lg">
            <IconComponent className="w-5 h-5" />
          </div>
          <div>
            <CardTitle className="text-lg">{capability.name}</CardTitle>
            <p className="text-sm text-muted-foreground font-mono">
              {capability.id}
            </p>
          </div>
        </div>
        <Badge variant={getStatusBadgeVariant(capability.status)}>
          {getStatusLabel(capability.status)}
        </Badge>
      </CardHeader>
      <CardContent>
        <p className="text-sm text-muted-foreground mb-4">
          {capability.description}
        </p>
        {capability.category && (
          <Badge variant="outline" className="text-xs">
            {capability.category}
          </Badge>
        )}
      </CardContent>
    </Card>
  );
}

export default function CapabilitiesPage() {
  const { data: capabilities, isLoading, error } = useCapabilities();

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">
          Error loading capabilities: {error.message}
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="mb-6">
        <h1 className="text-2xl font-bold">Capabilities</h1>
        <p className="text-muted-foreground">
          Capabilities add functionality to agents - tools, system prompt
          additions, and behavior modifications.
        </p>
      </div>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[...Array(4)].map((_, i) => (
            <Card key={i}>
              <CardHeader>
                <Skeleton className="h-6 w-3/4" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-4 w-full mb-2" />
                <Skeleton className="h-4 w-2/3" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : capabilities?.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-muted-foreground">No capabilities available</p>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {capabilities?.map((capability) => (
            <CapabilityCard key={capability.id} capability={capability} />
          ))}
        </div>
      )}
    </div>
  );
}
