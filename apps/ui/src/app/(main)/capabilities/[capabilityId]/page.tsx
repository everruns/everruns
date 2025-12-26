"use client";

import { use } from "react";
import { useCapability } from "@/hooks";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import {
  ArrowLeft,
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  Calculator,
  Globe,
  ListChecks,
  LucideIcon,
  Wrench,
  FileText,
  Code,
} from "lucide-react";
import type { CapabilityStatus, ToolDefinition } from "@/lib/api/types";

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

function ToolCard({ tool }: { tool: ToolDefinition }) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-2">
            <div className="p-1.5 bg-muted rounded">
              <Wrench className="w-4 h-4" />
            </div>
            <div>
              <CardTitle className="text-base font-mono">{tool.name}</CardTitle>
            </div>
          </div>
          {tool.policy && (
            <Badge
              variant={tool.policy === "auto" ? "default" : "secondary"}
              className="text-xs"
            >
              {tool.policy === "auto" ? "Auto" : "Requires Approval"}
            </Badge>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">{tool.description}</p>

        {tool.parameters && Object.keys(tool.parameters).length > 0 && (
          <div>
            <h4 className="text-sm font-medium mb-2 flex items-center gap-2">
              <Code className="w-4 h-4" />
              Parameters
            </h4>
            <pre className="text-xs bg-muted p-3 rounded-md overflow-x-auto">
              {JSON.stringify(tool.parameters, null, 2)}
            </pre>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export default function CapabilityDetailPage({
  params,
}: {
  params: Promise<{ capabilityId: string }>;
}) {
  const { capabilityId } = use(params);
  const { data: capability, isLoading, error } = useCapability(capabilityId);

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-4 w-24 mb-6" />
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (error || !capability) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500 mb-4">
          {error ? `Error loading capability: ${error.message}` : "Capability not found"}
        </div>
        <Link href="/capabilities" className="text-blue-500 hover:underline">
          Back to capabilities
        </Link>
      </div>
    );
  }

  const IconComponent = capability.icon
    ? iconMap[capability.icon] || CircleOff
    : CircleOff;

  const toolDefinitions = capability.tool_definitions || [];

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/capabilities"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Capabilities
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div className="flex items-center gap-4">
          <div className="p-3 bg-muted rounded-lg">
            <IconComponent className="w-6 h-6" />
          </div>
          <div>
            <h1 className="text-2xl font-bold flex items-center gap-3">
              {capability.name}
              <Badge variant={getStatusBadgeVariant(capability.status)}>
                {getStatusLabel(capability.status)}
              </Badge>
            </h1>
            <p className="text-muted-foreground font-mono text-sm">
              {capability.id}
            </p>
          </div>
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2 space-y-6">
          {/* Description Card */}
          <Card>
            <CardHeader>
              <CardTitle>Description</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="text-muted-foreground">{capability.description}</p>
            </CardContent>
          </Card>

          {/* System Prompt Card */}
          {capability.system_prompt && (
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <FileText className="w-5 h-5" />
                  System Prompt Addition
                </CardTitle>
              </CardHeader>
              <CardContent>
                <MarkdownDisplay content={capability.system_prompt} />
              </CardContent>
            </Card>
          )}

          {/* Tools Section */}
          {toolDefinitions.length > 0 && (
            <div className="space-y-4">
              <h2 className="text-lg font-semibold flex items-center gap-2">
                <Wrench className="w-5 h-5" />
                Tools ({toolDefinitions.length})
              </h2>
              <div className="space-y-4">
                {toolDefinitions.map((tool) => (
                  <ToolCard key={tool.name} tool={tool} />
                ))}
              </div>
            </div>
          )}

          {/* No contributions message */}
          {!capability.system_prompt && toolDefinitions.length === 0 && (
            <Card className="p-8 text-center">
              <CircleOff className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No contributions</h3>
              <p className="text-muted-foreground">
                This capability does not contribute any system prompt additions or tools.
              </p>
            </Card>
          )}
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div>
                <p className="text-sm font-medium">Status</p>
                <Badge
                  variant={getStatusBadgeVariant(capability.status)}
                  className="mt-1"
                >
                  {getStatusLabel(capability.status)}
                </Badge>
              </div>

              {capability.category && (
                <div>
                  <p className="text-sm font-medium">Category</p>
                  <Badge variant="outline" className="mt-1">
                    {capability.category}
                  </Badge>
                </div>
              )}

              <div>
                <p className="text-sm font-medium">ID</p>
                <p className="text-sm text-muted-foreground font-mono">
                  {capability.id}
                </p>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-base">Summary</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-sm">
                  <FileText className="h-4 w-4 text-muted-foreground" />
                  <span>System Prompt</span>
                </div>
                <Badge variant={capability.system_prompt ? "default" : "outline"}>
                  {capability.system_prompt ? "Yes" : "No"}
                </Badge>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-sm">
                  <Wrench className="h-4 w-4 text-muted-foreground" />
                  <span>Tools</span>
                </div>
                <span className="font-medium">{toolDefinitions.length}</span>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
