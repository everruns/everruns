"use client";

import { useCapabilities } from "@/hooks";
import { Header } from "@/components/layout/header";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import Link from "next/link";
import {
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  Calculator,
  Globe,
  ListChecks,
  LucideIcon,
  CheckCircle2,
  AlertCircle,
  Info,
} from "lucide-react";
import type { Capability, CapabilityStatus } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
  calculator: Calculator,
  globe: Globe,
  "list-checks": ListChecks,
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
    <Link href={`/capabilities/${capability.id}`}>
      <Card className="hover:shadow-md transition-shadow cursor-pointer h-full">
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
    </Link>
  );
}

function CapabilitySummary({ capabilities }: { capabilities: Capability[] }) {
  const available = capabilities.filter((c) => c.status === "available").length;
  const comingSoon = capabilities.filter((c) => c.status === "coming_soon").length;
  const deprecated = capabilities.filter((c) => c.status === "deprecated").length;

  const categories = [...new Set(capabilities.map((c) => c.category).filter(Boolean))];

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Summary</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm">
              <CheckCircle2 className="h-4 w-4 text-green-500" />
              <span>Available</span>
            </div>
            <span className="font-medium">{available}</span>
          </div>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm">
              <Clock className="h-4 w-4 text-yellow-500" />
              <span>Coming Soon</span>
            </div>
            <span className="font-medium">{comingSoon}</span>
          </div>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm">
              <AlertCircle className="h-4 w-4 text-muted-foreground" />
              <span>Deprecated</span>
            </div>
            <span className="font-medium">{deprecated}</span>
          </div>
        </CardContent>
      </Card>

      {categories.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Categories</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex flex-wrap gap-2">
              {categories.map((category) => (
                <Badge key={category} variant="outline">
                  {category}
                </Badge>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Info className="h-4 w-4" />
            About Capabilities
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            Capabilities add functionality to agents through tools, system prompt
            additions, and behavior modifications. Enable capabilities on individual
            agents to customize their behavior.
          </p>
        </CardContent>
      </Card>
    </div>
  );
}

export default function CapabilitiesPage() {
  const { data: capabilities, isLoading, error } = useCapabilities();

  if (error) {
    return (
      <>
        <Header
          title="Capabilities"
          description="Capabilities add functionality to agents - tools, system prompt additions, and behavior modifications."
        />
        <div className="p-6">
          <div className="text-red-500">
            Error loading capabilities: {error.message}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <Header
        title="Capabilities"
        description="Capabilities add functionality to agents - tools, system prompt additions, and behavior modifications."
      />

      <div className="p-6">
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2">
            {isLoading ? (
              <div className="grid gap-4 md:grid-cols-2">
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
              <Card className="p-8 text-center">
                <CircleOff className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
                <h3 className="text-lg font-medium mb-2">No capabilities available</h3>
                <p className="text-muted-foreground">
                  Capabilities will appear here once they are configured.
                </p>
              </Card>
            ) : (
              <div className="grid gap-4 md:grid-cols-2">
                {capabilities?.map((capability) => (
                  <CapabilityCard key={capability.id} capability={capability} />
                ))}
              </div>
            )}
          </div>

          <div className="space-y-6">
            {isLoading ? (
              <>
                <Card>
                  <CardHeader>
                    <Skeleton className="h-5 w-24" />
                  </CardHeader>
                  <CardContent className="space-y-3">
                    <Skeleton className="h-4 w-full" />
                    <Skeleton className="h-4 w-full" />
                    <Skeleton className="h-4 w-full" />
                  </CardContent>
                </Card>
                <Card>
                  <CardHeader>
                    <Skeleton className="h-5 w-32" />
                  </CardHeader>
                  <CardContent>
                    <Skeleton className="h-16 w-full" />
                  </CardContent>
                </Card>
              </>
            ) : (
              <CapabilitySummary capabilities={capabilities || []} />
            )}
          </div>
        </div>
      </div>
    </>
  );
}
