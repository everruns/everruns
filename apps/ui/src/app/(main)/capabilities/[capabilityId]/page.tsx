"use client";

import { use } from "react";
import { useCapability, useCapabilities, usePageTitle } from "@/hooks";
import Link from "next/link";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import {
  CircleOff,
  Wrench,
  FileText,
  Code,
  Link as LinkIcon,
  ExternalLink,
  Bot,
  Layers,
} from "lucide-react";
import type { CapabilityStatus, ToolDefinition } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { getCapabilityStatusBadgeVariant } from "@/lib/status-utils";
import { formatCountLabel } from "@/lib/formatting";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";

const DOCS_BASE_URL = "https://dev.everruns.com/capabilities";

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
          {"policy" in tool && tool.policy && (
            <Badge variant={tool.policy === "auto" ? "default" : "secondary"} className="text-xs">
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
            <pre className="text-xs bg-muted p-3 overflow-x-auto">
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
  const { locale } = useLocale();
  const { data: capability, isLoading, error } = useCapability(capabilityId);
  usePageTitle(capability ? localizedCapabilityName(capability, locale) : null, "Capability");
  const { data: allCapabilities } = useCapabilities();

  // Create a map of capability ID to capability for resolving dependency names
  const capabilityMap = new Map((allCapabilities || []).map((c) => [c.id, c]));

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
      <ResourceNotFound
        title={error ? "Capability unavailable" : "Capability not found"}
        description={
          error
            ? `The capability could not be loaded: ${error.message}`
            : "This capability may have been deleted, moved to another organization, or the URL may be wrong."
        }
        backHref="/capabilities"
        backLabel="Back to capabilities"
        resourceId={capabilityId}
      />
    );
  }

  const toolDefinitions = capability.tool_definitions || [];

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Capabilities", href: "/capabilities" },
          { label: localizedCapabilityName(capability, locale) },
        ]}
      />

      <PageMasthead
        icon={<CapabilityIcon icon={capability.icon} />}
        entityId={capability.id}
        title={localizedCapabilityName(capability, locale)}
        badges={
          <Badge variant={getCapabilityStatusBadgeVariant(capability.status)}>
            {getStatusLabel(capability.status)}
          </Badge>
        }
        meta={
          <>
            <span className="inline-flex items-center gap-1">
              <Bot className="h-3.5 w-3.5" />
              {formatCountLabel(capability.agent_count ?? 0, "agent")}
            </span>
            <span className="inline-flex items-center gap-1">
              <Layers className="h-3.5 w-3.5" />
              {formatCountLabel(capability.harness_count ?? 0, "harness", "harnesses")}
            </span>
            {capability.category && (
              <span>
                Category <span className="text-foreground">{capability.category}</span>
              </span>
            )}
          </>
        }
        actions={
          capability.docs_slug && (
            <a
              href={`${DOCS_BASE_URL}/${capability.docs_slug}/`}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
            >
              View documentation
              <ExternalLink className="w-3.5 h-3.5" />
            </a>
          )
        }
      />

      <PageColumns>
        <PageMain>
          {/* Description Card */}
          <Card>
            <CardHeader>
              <CardTitle>Description</CardTitle>
            </CardHeader>
            <CardContent>
              <InlineStreamdownMessage className="text-muted-foreground">
                {localizedCapabilityDescription(capability, locale)}
              </InlineStreamdownMessage>
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
        </PageMain>

        {/* Sidebar */}
        <PageRail>
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div>
                <p className="text-sm font-medium">Status</p>
                <Badge
                  variant={getCapabilityStatusBadgeVariant(capability.status)}
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

              {capability.dependencies && capability.dependencies.length > 0 && (
                <div>
                  <p className="text-sm font-medium flex items-center gap-1.5">
                    <LinkIcon className="w-3.5 h-3.5" />
                    Dependencies
                  </p>
                  <div className="mt-1.5 space-y-1">
                    {capability.dependencies.map((depId) => {
                      const depCap = capabilityMap.get(depId);
                      return (
                        <Link
                          key={depId}
                          href={`/capabilities/${depId}`}
                          className="block text-sm text-primary hover:underline"
                        >
                          {depCap ? localizedCapabilityName(depCap, locale) : depId}
                        </Link>
                      );
                    })}
                  </div>
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-base">Summary</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-sm">
                  <Bot className="h-4 w-4 text-muted-foreground" />
                  <span>Agents</span>
                </div>
                <span className="font-medium">{capability.agent_count ?? 0}</span>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-sm">
                  <Layers className="h-4 w-4 text-muted-foreground" />
                  <span>Harnesses</span>
                </div>
                <span className="font-medium">{capability.harness_count ?? 0}</span>
              </div>
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
        </PageRail>
      </PageColumns>

      <PageFooter>
        <BackLink href="/capabilities">Back to Capabilities</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
